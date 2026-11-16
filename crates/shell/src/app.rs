//! Application state and behaviour. Owns every screen's navigation state and the geometry of all
//! tiles (in design pixels on a 1920×1080 canvas), and pushes them to the Slint window in `render()`.
//! All input (controller, keyboard, mouse) arrives here as a `NavAction` or a pointer event.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cs_core::library::ImportedGame;
use cs_core::{
    AppPaths, Avatar, Background, Config, Game, GameEntry, Library, Profile, ProfileStore, Provider, Service,
};
use cs_input::{ControllerInfo, NavAction};
use cs_library::accounts::{Account, AccountStore};
use cs_library::playnite;
use cs_library::tracker::{self, TrackerEvent, TrackingHandle};
use cs_platform::process::SystemProcesses;
use slint::{Color, ComponentHandle, Image, Model, SharedString, VecModel};

use crate::files::{EntryKind, FbOutcome, FileBrowser, Filter};
use crate::images::{self, ImageCache, Pixels, Size};
use crate::nav::{Rect, list_move, spatial_move};
use crate::osk::{Osk, OskOutcome};
use crate::social::{self, Friend, Party, Presence, SocialBackend};
use crate::ui::{
    AchievementRow, AppWindow, AudioRow, ProfileStat, ChatLine, GameButton, GuideItem, GuideTab, Hint, KeyView, ListItem, MenuItem,
    Modal, PickerItem, ProfileItem, Screen, SettingTile, SidebarItem, Tile,
};
use crate::updater;
use crate::weblogin::{FocusInfo, Pkce, WebEvent, WebLogin};

/// Ignore mouse hover for this long after controller/keyboard input, so tiles moving under a resting
/// cursor don't steal focus.
const HOVER_GRACE: Duration = Duration::from_millis(800);
const TOAST_TIME: Duration = Duration::from_secs(3);
const RECENT_COUNT: u32 = 12;

/// Xbox-style accent palette offered in the picker (the last slot opens a hex entry).
const ACCENTS: [(&str, &str); 11] = [
    ("#107C10", "Green"),
    ("#0078D7", "Blue"),
    ("#00B7C3", "Teal"),
    ("#8764B8", "Purple"),
    ("#E3008C", "Pink"),
    ("#A4262C", "Red"),
    ("#F7630C", "Orange"),
    ("#C19C00", "Gold"),
    ("#498205", "Moss"),
    ("#515C6B", "Steel"),
    ("#744DA9", "Violet"),
];
const BUILTIN_AVATARS: usize = 8;

// Tile styles, see `Tile.style` in ui/components.slint.
const STYLE_ART: i32 = 0;
const STYLE_SOLID: i32 = 1;
const STYLE_DARK: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shortcut {
    Settings,
    AddGame,
    Search,
    SwitchProfile,
    Power,
}

impl Shortcut {
    const ALL: [Shortcut; 5] =
        [Shortcut::Settings, Shortcut::AddGame, Shortcut::Search, Shortcut::SwitchProfile, Shortcut::Power];

    fn glyph(self) -> &'static str {
        match self {
            Shortcut::Settings => "\u{E713}",
            Shortcut::AddGame => "\u{E710}",
            Shortcut::Search => "\u{E721}",
            Shortcut::SwitchProfile => "\u{E748}",
            Shortcut::Power => "\u{E7E8}",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Shortcut::Settings => "Settings",
            Shortcut::AddGame => "Add a game",
            Shortcut::Search => "Search",
            Shortcut::SwitchProfile => "Switch profile",
            Shortcut::Power => "Power",
        }
    }
}

#[derive(Debug, Clone)]
enum HeroItem {
    Game(GameEntry),
    Shortcut(Shortcut),
    Library,
}

/// A tile in the Home "hero" grid.
struct HeroTile {
    rect: Rect,
    item: HeroItem,
    /// Wide recent tiles use landscape art and show a subtitle.
    wide: bool,
    /// Caption always visible (big and wide tiles) vs. only when focused (squares).
    caption: bool,
}

/// A Home section below the hero grid (Pinned, Steam, ...).
struct Section {
    title: String,
    entries: Vec<GameEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Splash,
    Profiles,
    Home,
    Library,
    Settings,
    Files,
    Search,
    About,
    SignIn,
    Game,
    MyProfile,
}

/// The game hub page.
struct GamePageState {
    game_id: String,
    focus: usize,
    achievements: Vec<cs_library::achievements::Achievement>,
    ret: View,
}

const GAME_BUTTONS: usize = 3;

/// Actions on your profile page (glyph, label).
const PROFILE_BUTTONS: [(&str, &str); 4] = [
    ("\u{E771}", "Customize profile"),
    ("\u{E70F}", "Edit bio"),
    ("\u{E7FC}", "Choose showcase games"),
    ("\u{E72D}", "Share profile"),
];
/// Most games shown on the profile page.
const SHOWCASE_MAX: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sort {
    Name,
    Recent,
    Playtime,
}

impl Sort {
    const ALL: [Sort; 3] = [Sort::Name, Sort::Recent, Sort::Playtime];

    fn label(self) -> &'static str {
        match self {
            Sort::Name => "Sort A-Z",
            Sort::Recent => "Sort by recently played",
            Sort::Playtime => "Sort by most played",
        }
    }
}

/// My games & apps sidebar entries: filters (all games, one per installed library add-on, custom games),
/// then hidden games and the add-on actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LibEntry {
    All,
    Installed,
    Provider(Provider),
    Hidden,
    ManageAddons,
    Update,
}

/// The Discord application (Social SDK) Win2Console uses; `Config::discord_app_id` overrides it.
const DEFAULT_DISCORD_APP_ID: &str = "1571675582024908851";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LibZone {
    Sidebar,
    /// 0 = Sort, 1 = Add a game
    Controls(usize),
    Grid,
}

const SETTINGS_CATEGORIES: [&str; 5] = ["Profile", "Accounts", "Library", "System", "Power"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    Name,
    Avatar,
    Accent,
    Background,
    SwitchProfile,
    DeleteProfile,
    LinkDiscord,
    SteamAccount,
    Music,
    AppTitle,
    AppIcon,
    StartupAnimation,
    MenuSize,
    Updates,
    ShowHidden,
    Rescan,
    AddGame,
    StartWithWindows,
    About,
    Sleep,
    Restart,
    ShutDown,
    ExitToDesktop,
    Quit,
}

fn settings_for(category: usize) -> &'static [Setting] {
    match category {
        0 => &[
            Setting::Name,
            Setting::Avatar,
            Setting::Accent,
            Setting::Background,
            Setting::SwitchProfile,
            Setting::DeleteProfile,
        ],
        1 => &[Setting::LinkDiscord, Setting::SteamAccount],
        2 => &[Setting::ShowHidden, Setting::Rescan, Setting::AddGame],
        3 => &[
            Setting::Music,
            Setting::MenuSize,
            Setting::AppTitle,
            Setting::AppIcon,
            Setting::StartupAnimation,
            Setting::Updates,
            Setting::StartWithWindows,
            Setting::About,
            Setting::ExitToDesktop,
        ],
        _ => &[Setting::Sleep, Setting::Restart, Setting::ShutDown, Setting::ExitToDesktop, Setting::Quit],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtSlot {
    Cover,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MenuAction {
    Play(String),
    TogglePin(String),
    ToggleHidden(String),
    SetArt(String, ArtSlot),
    Remove(String),
    ConfirmRemove(String),
    ConfirmDeleteProfile,
    SortBy(Sort),
    Guide(GuideAction),
    Sleep,
    Restart,
    ShutDown,
    ExitToDesktop,
    QuitGame,
    Details(String),
    ChooseAppIcon,
    ToggleShowcase(String),
    ResetAppIcon,
    CheckUpdates,
    InstallUpdate,
    ToggleAutoUpdate,
    AnswerRequest(u64, bool),
    UiScale(u32),
    Startup(StartupChoice),
    /// Account name and persona, or `None` to unlink.
    SetSteamAccount(Option<(String, String)>),
    Close,
}

struct MenuEntry {
    glyph: &'static str,
    label: String,
    action: MenuAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    Accent { onboarding: bool },
    Avatar { onboarding: bool },
    Background,
    Captures { count: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OskPurpose {
    NewProfile,
    RenameProfile,
    CustomAccent { onboarding: bool },
    NewGameName { exe: PathBuf },
    AppTitle,
    AddFriend,
    Bio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FilePurpose {
    AddGame,
    Art { game_id: String, slot: ArtSlot },
    Avatar,
    Background,
    AppIcon,
    StartupVideo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GuideAction {
    Home,
    Library,
    Game(String),
    Settings(usize),
    SwitchProfile,
    MyProfile,
    Search,
    AddGame,
    Power,
    LinkDiscord,
    Friend(u64),
    OpenChat(u64),
    StartParty,
    JoinParty(u64),
    ToggleMute,
    ToggleDeafen,
    LeaveParty,
    Resume,
    AddFriend,
    FriendRequest(u64),
    Chats,
    Details(String),
    StartRecording,
    StopRecording,
    OpenCapture(usize),
    Captures,
    Store,
    GamePass,
    Screenshot,
    Audio,
    QuitGame,
    ClearNotices,
    Keyboard,
    None,
}

/// Guide tabs, in order (the last one shows the profile picture).
/// "trophy" is drawn as a shape (the icon font has no trophy).
const GUIDE_TABS: [&str; 7] = ["\u{E80F}", "\u{E716}", "\u{E8BD}", "trophy", "\u{E722}", "\u{EA8F}", ""];
const TAB_PEOPLE: usize = 1;
const TAB_PARTY: usize = 2;
const TAB_ACHIEVEMENTS: usize = 3;
const TAB_CAPTURE: usize = 4;
const TAB_NOTIFICATIONS: usize = 5;
/// Notifications kept for the Guide (they're cleared when Win2Console closes).
const NOTICES_MAX: usize = 50;

/// Something that happened, listed in the Guide's notifications tab.
#[derive(Debug, Clone)]
struct Notice {
    glyph: &'static str,
    title: String,
    body: String,
    /// A person's picture or an achievement's icon.
    image: Option<PathBuf>,
    /// People get a round picture; achievements a square icon.
    round: bool,
    at: i64,
    action: GuideAction,
}

/// A conversation open in the Guide.
struct GuideChat {
    peer: u64,
    /// Keyboard open for writing a message.
    compose: Option<Osk>,
    /// How many messages back from the latest the view is scrolled.
    scroll: usize,
}

struct GuideEntry {
    y: f32,
    glyph: &'static str,
    label: String,
    subtitle: String,
    image: Option<PathBuf>,
    solid: bool,
    thumb: bool,
    divider_above: bool,
    round: bool,
    presence: i32,
    action: GuideAction,
    /// Other profiles' pictures (the "Switch profile" row).
    others: Vec<Profile>,
    more: String,
    /// Shows your own picture in place of the icon.
    profile_row: bool,
    /// The label is a gamerscore (shown after the gamerscore mark).
    score: bool,
}

const GUIDE_BUTTONS: [(&str, GuideAction); 5] = [
    ("\u{E721}", GuideAction::Search),
    ("\u{E765}", GuideAction::Keyboard),
    ("\u{E748}", GuideAction::SwitchProfile),
    ("\u{E767}", GuideAction::Audio),
    ("\u{E7E8}", GuideAction::Power),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupChoice {
    BuiltIn,
    Video,
    Off,
}

/// A screen recording in progress (ffmpeg).
struct Recording {
    child: std::process::Child,
    started: Instant,
    path: PathBuf,
}

/// The Guide's Achievements tab: recently played games with achievements, and the latest unlocks.
struct AchGuide {
    at: Instant,
    games: Vec<(GameEntry, cs_library::achievements::Summary)>,
    recent: Vec<(Option<String>, cs_library::achievements::RecentUnlock)>,
}

fn is_video(p: &Path) -> bool {
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4"))
}

/// ffmpeg next to Win2Console, on PATH, or where winget puts it.
fn find_ffmpeg() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let path_dirs = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    let winget = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Microsoft").join("WinGet"));
    let mut dirs: Vec<PathBuf> = exe_dir.into_iter().chain(path_dirs).collect();
    if let Some(w) = &winget {
        dirs.push(w.join("Links"));
        // Portable packages: Packages\Gyan.FFmpeg_*\ffmpeg-*\bin
        for pkg in std::fs::read_dir(w.join("Packages")).into_iter().flatten().flatten() {
            for sub in std::fs::read_dir(pkg.path()).into_iter().flatten().flatten() {
                dirs.push(sub.path().join("bin"));
            }
        }
    }
    dirs.into_iter().map(|d| d.join("ffmpeg.exe")).find(|p| p.is_file())
}

/// The launch screen shown while a game starts.
struct Launching {
    shown: Instant,
    /// When the game's process appeared.
    started: Option<Instant>,
    closing: Option<Instant>,
    last_focus_try: Instant,
}

const VIEW_ANIM: f32 = 0.28;
const MODAL_ANIM: f32 = 0.2;
const SECTION_ANIM: f32 = 0.25;
const LAUNCH_FADE: f32 = 0.3;
/// The startup animation fades to black, then the profile picker fades in from black.
const INTRO_FADE_OUT: f32 = 0.6;
const INTRO_FADE_IN: f32 = 1.0;
/// The launch screen stays at least this long, so it doesn't just flash.
const LAUNCH_MIN: f32 = 1.5;

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The Guide opened over a running game: a frozen capture of the screen sits behind it.
struct InGame {
    capture: Option<Arc<cs_platform::capture::Capture>>,
    backdrop: Option<slint::Image>,
    /// The backdrop has been handed to the UI.
    shown: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioKind {
    Master,
    Game,
    Party,
    MicVolume,
    NoiseSuppression,
    Microphone,
    Speakers,
}

#[derive(Debug, Clone)]
struct AudioSource {
    kind: AudioKind,
    level: f32,
    muted: bool,
    /// Set for option rows (devices, on/off) instead of a slider.
    value: Option<String>,
}

const VOLUME_STEP: f32 = 0.05;

/// Power screen options, in rows of 3.
const POWER_ITEMS: [(&str, &str, MenuAction); 4] = [
    ("\u{E7E8}", "Turn off PC", MenuAction::ShutDown),
    ("\u{E777}", "Restart PC", MenuAction::Restart),
    ("\u{E708}", "Sleep", MenuAction::Sleep),
    ("\u{E89F}", "Exit to desktop", MenuAction::ExitToDesktop),
];
const APP_ICON: &str = "app-icon.png";

enum ModalState {
    Guide {
        tab: usize,
        focus: usize,
    },
    Audio {
        focus: usize,
        sources: Vec<AudioSource>,
        game_pids: Vec<u32>,
    },
    Viewer {
        index: usize,
    },
    /// A friend's card next to the Guide (which stays on `tab` / `guide_focus`).
    Friend {
        id: u64,
        focus: usize,
        tab: usize,
        guide_focus: usize,
    },
    /// Recent chats card next to the Guide.
    Chats {
        focus: usize,
        tab: usize,
        guide_focus: usize,
    },
    Power {
        focus: usize,
    },
    Menu {
        title: String,
        message: String,
        entries: Vec<MenuEntry>,
        focus: usize,
    },
    Picker {
        kind: PickerKind,
        focus: usize,
    },
    Keyboard {
        osk: Osk,
        purpose: OskPurpose,
    },
}

struct SearchState {
    osk: Osk,
    results: Vec<GameEntry>,
    /// `None` = focus is on the keyboard.
    focus: Option<usize>,
    ret: View,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignInZone {
    Page,
    Keyboard,
}

/// An open sign-in page for linking `service`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignInTarget {
    Discord,
    /// Plain controller browsing (Store, Game Pass).
    Browse {
        title: &'static str,
        glyph: &'static str,
    },
}

struct SignInState {
    target: SignInTarget,
    web: WebLogin,
    pkce: Pkce,
    zone: SignInZone,
    /// Latest focused element reported by the page.
    field: FocusInfo,
    osk: Osk,
    ret: View,
}

/// Discord's desktop OAuth redirect for the Social SDK.
const DISCORD_REDIRECT: &str = "http://127.0.0.1/callback";

struct Running {
    game_id: String,
    profile: uuid::Uuid,
    handle: TrackingHandle,
}

/// Persistent models, updated in place (row diffs) so moving focus never recreates tiles.
struct Models {
    hints: Rc<VecModel<Hint>>,
    profiles: Rc<VecModel<ProfileItem>>,
    hero: Rc<VecModel<Tile>>,
    home_sections: Rc<VecModel<SharedString>>,
    section_tiles: Rc<VecModel<Tile>>,
    lib_categories: Rc<VecModel<SidebarItem>>,
    games: Rc<VecModel<Tile>>,
    settings_categories: Rc<VecModel<SharedString>>,
    settings_tiles: Rc<VecModel<SettingTile>>,
    list_items: Rc<VecModel<ListItem>>,
    guide_tabs: Rc<VecModel<GuideTab>>,
    guide_items: Rc<VecModel<GuideItem>>,
    guide_buttons: Rc<VecModel<SharedString>>,
    menu_items: Rc<VecModel<MenuItem>>,
    picker_items: Rc<VecModel<PickerItem>>,
    kb_keys: Rc<VecModel<KeyView>>,
    search_results: Rc<VecModel<Tile>>,
    chat_lines: Rc<VecModel<ChatLine>>,
    audio_rows: Rc<VecModel<AudioRow>>,
}

impl Models {
    fn new(ui: &AppWindow) -> Self {
        fn m<T: Clone + 'static>() -> Rc<VecModel<T>> {
            Rc::new(VecModel::default())
        }
        let models = Self {
            hints: m(),
            profiles: m(),
            hero: m(),
            home_sections: m(),
            section_tiles: m(),
            lib_categories: m(),
            games: m(),
            settings_categories: m(),
            settings_tiles: m(),
            list_items: m(),
            guide_tabs: m(),
            guide_items: m(),
            guide_buttons: m(),
            menu_items: m(),
            picker_items: m(),
            kb_keys: m(),
            search_results: m(),
            chat_lines: m(),
            audio_rows: m(),
        };
        ui.set_hints(models.hints.clone().into());
        ui.set_profiles(models.profiles.clone().into());
        ui.set_hero(models.hero.clone().into());
        ui.set_home_sections(models.home_sections.clone().into());
        ui.set_section_tiles(models.section_tiles.clone().into());
        ui.set_lib_categories(models.lib_categories.clone().into());
        ui.set_games(models.games.clone().into());
        ui.set_settings_categories(models.settings_categories.clone().into());
        ui.set_settings_tiles(models.settings_tiles.clone().into());
        ui.set_list_items(models.list_items.clone().into());
        ui.set_guide_tabs(models.guide_tabs.clone().into());
        ui.set_guide_items(models.guide_items.clone().into());
        ui.set_guide_buttons(models.guide_buttons.clone().into());
        ui.set_menu_items(models.menu_items.clone().into());
        ui.set_picker_items(models.picker_items.clone().into());
        ui.set_kb_keys(models.kb_keys.clone().into());
        ui.set_search_results(models.search_results.clone().into());
        ui.set_chat_lines(models.chat_lines.clone().into());
        ui.set_audio_rows(models.audio_rows.clone().into());
        models
    }
}

/// Updates `model` to `items`, touching only rows that changed (or resetting if the length differs).
fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, items: Vec<T>) {
    if model.row_count() != items.len() {
        model.set_vec(items);
        return;
    }
    for (i, item) in items.into_iter().enumerate() {
        if model.row_data(i).as_ref() != Some(&item) {
            model.set_row_data(i, item);
        }
    }
}

pub struct App {
    ui: slint::Weak<AppWindow>,
    paths: AppPaths,
    config: Config,
    store: ProfileStore,
    library: Library,
    images: ImageCache,
    models: Models,

    profiles: Vec<Profile>,
    current: Option<Profile>,
    view: View,
    /// Where B goes from Settings / About.
    back_to: View,
    modal: Option<ModalState>,

    profiles_focus: usize,

    /// 0 = hero grid, k = sections[k - 1]
    home_section: usize,
    hero_focus: usize,
    section_focus: usize,
    hero: Vec<HeroTile>,
    sections: Vec<Section>,

    lib_category: usize,
    lib_zone: LibZone,
    lib_focus: usize,
    lib_sort: Sort,
    lib_games: Vec<GameEntry>,
    lib_counts: Vec<usize>,

    settings_category: usize,
    settings_in_content: bool,
    settings_focus: usize,

    files: Option<(FileBrowser, FilePurpose, View)>,
    search: Option<SearchState>,
    signin: Option<SignInState>,

    /// Playnite (the library backend): its program, the export's last-seen modification time, and a watcher
    /// that re-imports when the export changes.
    playnite_exe: Option<PathBuf>,
    export_mtime: Option<std::time::SystemTime>,
    sync_started: Instant,
    export_timer: Rc<slint::Timer>,

    social: Option<Box<dyn SocialBackend>>,
    social_status: social::Status,
    friends: Vec<Friend>,
    /// People we have chats with, as Discord described them (they may not be friends).
    users: HashMap<u64, Friend>,
    notices: Vec<Notice>,
    /// "Exit to desktop": Win2Console waits in the background, the controller works as a mouse, and the Guide
    /// button brings the Guide back.
    desktop: bool,
    /// The virtual keyboard that types into the game or app in front.
    vkeyboard: Option<(Osk, crate::ui::KeyboardWindow)>,
    /// Achievements already seen (game, name); `None` until the first look at the add-on's database.
    ach_seen: Option<std::collections::HashSet<(String, String)>>,
    ach_timer: Rc<slint::Timer>,
    /// Achievement pop-ups waiting to be shown, and the one on screen.
    ach_queue: std::collections::VecDeque<cs_library::achievements::RecentUnlock>,
    ach_popup: Option<crate::ui::AchievementWindow>,
    ach_popup_timer: Rc<slint::Timer>,
    /// Conversations by friend id, oldest message first.
    chats: HashMap<u64, Vec<social::Message>>,
    party: Option<Party>,
    party_invite: Option<(u64, String)>,
    guide_chat: Option<GuideChat>,
    /// Guide list scroll offset (design px).
    guide_scroll: f32,
    social_timer: Rc<slint::Timer>,
    chat_save_timer: Rc<slint::Timer>,
    splash_started: Instant,
    splash_timer: Rc<slint::Timer>,
    splash_ending: bool,
    splash_model: Rc<VecModel<crate::ui::SplashPetal>>,

    show_hidden: bool,
    scanning: bool,
    scan_summary: String,
    running: Option<Running>,
    ingame: Option<InGame>,
    image_timer: Rc<slint::Timer>,
    pub hotkey: Option<cs_platform::window::GuideHotkey>,
    home_hotkey_on: bool,
    /// Win2Console's window has focus (the ambient music pauses while tabbed out).
    window_focused: bool,
    gifs: std::cell::RefCell<crate::gifs::Gifs>,
    gif_timer: Rc<slint::Timer>,
    /// Transitions: when the page / modal last changed, and what they were.
    anim_timer: Rc<slint::Timer>,
    view_changed: Option<Instant>,
    modal_changed: Option<Instant>,
    section_changed: Option<Instant>,
    last_home_section: usize,
    last_view: View,
    last_modal: Option<std::mem::Discriminant<ModalState>>,
    launching: Option<Launching>,
    /// Screenshots in Pictures\Win2Console, newest first (Captures).
    captures: Vec<PathBuf>,
    captures_at: Option<Instant>,
    cursor_hidden: bool,
    update_state: crate::updater::State,
    update_info: Option<velopack::UpdateInfo>,
    /// Notification pop-up shown over games.
    notify: Option<crate::ui::NotifyWindow>,
    notify_timer: Rc<slint::Timer>,
    /// Incoming Discord friend requests.
    requests: Vec<Friend>,
    discord_audio: Option<social::AudioInfo>,
    /// Horizontal scroll of the Home row of squares (design px).
    hero_row_scroll: f32,
    /// Guide tab and row an action was chosen from (to return to).
    last_guide: (usize, usize),
    recording: Option<Recording>,
    ach_guide: Option<AchGuide>,
    /// A recording playing in the capture viewer (index, player).
    viewer_video: Option<(usize, crate::video::StartupVideo)>,
    /// Pictures from `%APPDATA%\Win2Console\avatars\`, offered as profile pictures.
    avatar_pack: Vec<PathBuf>,
    game_page: Option<GamePageState>,
    /// Your profile page: focus (buttons, then showcased games) and where Back goes.
    my_profile: (usize, View),
    store_feed: Vec<GameEntry>,
    gamepass_feed: Vec<GameEntry>,
    startup_video: Option<crate::video::StartupWindow>,
    /// Total Gamerscore from the Playnite Achievements add-on (`None` until loaded / not installed).
    gamerscore: Option<u32>,
    /// Skip loading images for tiles built while this is set (off-screen library rows).
    lazy_images: bool,
    controllers: Vec<ControllerInfo>,
    last_nav: Instant,
    toast_timer: Rc<slint::Timer>,
}

/// Posts work to the UI thread's `App`.
pub fn post(f: impl FnOnce(&mut App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        crate::with_app(f);
    });
}

impl App {
    pub fn new(ui: &AppWindow, paths: AppPaths) -> Result<Self, Box<dyn std::error::Error>> {
        paths.ensure()?;
        let config = Config::load(&paths.config_file());
        let store = ProfileStore::new(paths.profiles_dir());
        let library = Library::open(&paths.library_db())?;
        let profiles = store.list()?;
        let images = ImageCache::new(paths.root.join("cache").join("images"), |path, size, pixels: Option<Pixels>| {
            post(move |app| app.image_loaded(path, size, pixels))
        });
        let profiles_focus = config.last_profile.and_then(|id| profiles.iter().position(|p| p.id == id)).unwrap_or(0);

        Ok(Self {
            ui: ui.as_weak(),
            models: Models::new(ui),
            paths,
            config,
            store,
            library,
            images,
            profiles,
            current: None,
            view: View::Splash,
            back_to: View::Home,
            modal: None,
            profiles_focus,
            home_section: 0,
            hero_focus: 0,
            section_focus: 0,
            hero: Vec::new(),
            sections: Vec::new(),
            lib_category: 0,
            lib_zone: LibZone::Sidebar,
            lib_focus: 0,
            lib_sort: Sort::Name,
            lib_games: Vec::new(),
            lib_counts: Vec::new(),
            settings_category: 0,
            settings_in_content: false,
            settings_focus: 0,
            files: None,
            search: None,
            signin: None,
            playnite_exe: None,
            export_mtime: None,
            sync_started: Instant::now(),
            export_timer: Rc::new(slint::Timer::default()),
            social: None,
            social_status: social::Status::NotLinked,
            friends: Vec::new(),
            users: HashMap::new(),
            notices: Vec::new(),
            desktop: false,
            vkeyboard: None,
            ach_seen: None,
            ach_timer: Rc::new(slint::Timer::default()),
            ach_queue: Default::default(),
            ach_popup: None,
            ach_popup_timer: Rc::new(slint::Timer::default()),
            chats: HashMap::new(),
            party: None,
            party_invite: None,
            guide_chat: None,
            guide_scroll: 0.0,
            social_timer: Rc::new(slint::Timer::default()),
            chat_save_timer: Rc::new(slint::Timer::default()),
            splash_started: Instant::now(),
            splash_timer: Rc::new(slint::Timer::default()),
            splash_ending: false,
            splash_model: Rc::new(VecModel::default()),
            show_hidden: false,
            scanning: false,
            scan_summary: String::new(),
            running: None,
            ingame: None,
            image_timer: Rc::new(slint::Timer::default()),
            hotkey: None,
            home_hotkey_on: false,
            window_focused: true,
            gifs: std::cell::RefCell::new(crate::gifs::Gifs::new()),
            gif_timer: Rc::new(slint::Timer::default()),
            anim_timer: Rc::new(slint::Timer::default()),
            view_changed: None,
            modal_changed: None,
            section_changed: None,
            last_home_section: 0,
            last_view: View::Splash,
            last_modal: None,
            launching: None,
            captures: Vec::new(),
            captures_at: None,
            cursor_hidden: false,
            update_state: crate::updater::State::Idle,
            update_info: None,
            notify: None,
            notify_timer: Rc::new(slint::Timer::default()),
            requests: Vec::new(),
            discord_audio: None,
            hero_row_scroll: 0.0,
            last_guide: (0, 0),
            recording: None,
            ach_guide: None,
            viewer_video: None,
            avatar_pack: Vec::new(),
            game_page: None,
            my_profile: (0, View::Home),
            store_feed: Vec::new(),
            gamepass_feed: Vec::new(),
            startup_video: None,
            gamerscore: None,
            lazy_images: false,
            controllers: Vec::new(),
            last_nav: Instant::now(),
            toast_timer: Rc::new(slint::Timer::default()),
        })
    }

    fn ui(&self) -> AppWindow {
        self.ui.upgrade().expect("window alive while app runs")
    }

    fn profile_id(&self) -> Option<uuid::Uuid> {
        self.current.as_ref().map(|p| p.id)
    }

    /// The on-screen keyboard currently receiving text, if any.
    fn active_osk(&mut self) -> Option<&mut Osk> {
        if matches!(self.modal, Some(ModalState::Guide { .. }))
            && let Some(GuideChat { compose: Some(osk), .. }) = &mut self.guide_chat
        {
            return Some(osk);
        }
        match (&mut self.modal, &mut self.search, &mut self.signin) {
            (Some(ModalState::Keyboard { osk, .. }), _, _) => Some(osk),
            (None, Some(s), _) if self.view == View::Search && s.focus.is_none() => Some(&mut s.osk),
            (None, _, Some(si)) if self.view == View::SignIn && si.zone == SignInZone::Keyboard => Some(&mut si.osk),
            _ => None,
        }
    }

    // ================================================================ input

    /// Keyboard input from the window. Returns whether the key was used.
    pub fn on_key(&mut self, text: &str) -> bool {
        use slint::platform::Key;
        let is = |k: Key| text == SharedString::from(k).as_str();

        // Typing on a selected text field of a sign-in page goes into that field.
        if self.view == View::SignIn
            && self.modal.is_none()
            && let Some(si) = &mut self.signin
            && si.zone == SignInZone::Page
            && si.field.text
            && text.chars().count() == 1
            && !text.chars().all(char::is_control)
        {
            si.osk = Osk::new(si.field.label.clone(), &si.field.value, 256);
            si.zone = SignInZone::Keyboard;
        }

        // While a keyboard is open, the physical keyboard types into it.
        if let Some(osk) = self.active_osk() {
            if is(Key::Backspace) {
                osk.backspace();
                self.text_changed();
                return true;
            }
            if let Some(c) = text.chars().next().filter(|c| text.chars().count() == 1 && !c.is_control()) {
                osk.type_char(c);
                self.text_changed();
                return true;
            }
            if is(Key::Return) {
                return self.on_nav(NavAction::Menu);
            }
        }

        let action = if is(Key::UpArrow) {
            NavAction::Up
        } else if is(Key::DownArrow) {
            NavAction::Down
        } else if is(Key::LeftArrow) {
            NavAction::Left
        } else if is(Key::RightArrow) {
            NavAction::Right
        } else if is(Key::Return) || text == " " {
            NavAction::Accept
        } else if is(Key::Escape) || is(Key::Backspace) {
            NavAction::Back
        } else if is(Key::PageUp) {
            NavAction::PrevTab
        } else if is(Key::PageDown) {
            NavAction::NextTab
        } else if is(Key::Home) {
            NavAction::Guide
        } else {
            match text {
                "x" | "X" => NavAction::X,
                "y" | "Y" => NavAction::Y,
                "m" | "M" => NavAction::Menu,
                "v" | "V" => NavAction::View,
                _ => return false,
            }
        };
        self.on_nav(action)
    }

    pub fn on_controllers(&mut self, controllers: Vec<ControllerInfo>) {
        self.controllers = controllers;
        self.render();
    }

    pub fn on_nav(&mut self, action: NavAction) -> bool {
        tracing::debug!("nav {action:?} (modal: {})", self.modal.is_some());
        self.last_nav = Instant::now();
        if self.view == View::Splash {
            // Only A (Enter/Space on a keyboard) skips the startup animation; other buttons do nothing.
            if action == NavAction::Accept {
                self.end_splash();
            }
            return true;
        }
        // While playing, the controller belongs to the game: only the Guide button reaches Win2Console.
        if self.vkeyboard.is_some() {
            self.nav_vkeyboard(action);
            return true;
        }
        if (self.running.is_some() || self.desktop)
            && action != NavAction::Guide
            && !cs_platform::window::own_window_is_foreground()
        {
            return false;
        }
        if let Some(l) = &mut self.launching {
            if action == NavAction::Back && l.closing.is_none() {
                l.closing = Some(Instant::now());
                self.start_anim();
            }
            return true;
        }
        use crate::sounds::{Sound, play};
        match action {
            NavAction::Up | NavAction::Down | NavAction::Left | NavAction::Right => play(Sound::Move),
            NavAction::Accept => play(Sound::Select),
            NavAction::Back => play(Sound::Back),
            NavAction::Guide => play(Sound::Guide),
            _ => {}
        }
        if action == NavAction::Guide {
            self.toggle_guide();
        } else if self.modal.is_some() {
            self.nav_modal(action);
        } else {
            match self.view {
                View::Splash => {}
                View::Profiles => self.nav_profiles(action),
                View::Home => self.nav_home(action),
                View::Library => self.nav_library(action),
                View::Settings => self.nav_settings(action),
                View::Files => self.nav_files(action),
                View::Search => self.nav_search(action),
                View::SignIn => self.nav_signin(action),
                View::Game => self.nav_game(action),
                View::MyProfile => self.nav_my_profile(action),
                View::About => {
                    if action == NavAction::Back {
                        self.view = View::Settings;
                    }
                }
            }
        }
        // Over a game, backing out of a sub-menu returns to the Guide rather than the frozen backdrop.
        if self.ingame.is_some() && self.modal.is_none() {
            self.modal = Some(ModalState::Guide { tab: 0, focus: 0 });
        }
        self.render();
        true
    }

    /// Mouse hover (`activate == false`) moves focus; click (`true`) also activates.
    pub fn on_pointer(&mut self, area: &str, _row: i32, index: i32, activate: bool) {
        if !activate && self.last_nav.elapsed() < HOVER_GRACE {
            return;
        }
        let i = index.max(0) as usize;
        match area {
            "splash" => {
                if activate && self.view == View::Splash {
                    self.end_splash();
                }
                return;
            }
            "key" => {
                if activate {
                    let outcome = self.active_osk().map(|osk| osk.press(i));
                    if let Some(outcome) = outcome {
                        self.osk_outcome(outcome);
                    }
                } else if let Some(osk) = self.active_osk() {
                    osk.focus = i;
                }
                self.render();
                return;
            }
            "guide-tab" => {
                if let Some(ModalState::Guide { tab, focus }) = &mut self.modal {
                    *tab = i;
                    *focus = 0;
                }
                self.render();
                return;
            }
            "lib-category" => {
                self.select_category(i);
                self.lib_zone = LibZone::Sidebar;
                self.render();
                return;
            }
            "lib-control" => {
                self.lib_zone = LibZone::Controls(i);
                self.on_nav(NavAction::Accept);
                return;
            }
            "settings-category" => {
                self.settings_category = i.min(SETTINGS_CATEGORIES.len() - 1);
                self.settings_in_content = false;
                self.settings_focus = 0;
                self.render();
                return;
            }
            _ => {}
        }
        match (area, &mut self.modal) {
            ("menu", Some(ModalState::Menu { focus, .. }))
            | ("picker", Some(ModalState::Picker { focus, .. }))
            | ("guide", Some(ModalState::Guide { focus, .. }))
            | ("audio", Some(ModalState::Audio { focus, .. }))
            | ("power", Some(ModalState::Power { focus }))
            | ("friend", Some(ModalState::Friend { focus, .. }))
            | ("chats", Some(ModalState::Chats { focus, .. })) => *focus = i,
            ("profiles", None) => self.profiles_focus = i,
            ("game", None) => {
                if let Some(g) = &mut self.game_page {
                    g.focus = i;
                }
            }
            ("myprofile", None) => self.my_profile.0 = i,
            ("hero", None) => self.hero_focus = i,
            ("section", None) => self.section_focus = i,
            ("lib-game", None) => {
                if self.lib_zone != LibZone::Grid && !activate {
                    return; // hovering the grid while the sidebar is open shouldn't collapse it
                }
                self.lib_zone = LibZone::Grid;
                self.lib_focus = i;
            }
            ("settings-tile", None) => {
                self.settings_in_content = true;
                self.settings_focus = i;
            }
            ("list", None) => {
                if let Some((fb, _, _)) = &mut self.files {
                    fb.focus = i;
                }
            }
            ("search", None) => {
                if let Some(s) = &mut self.search {
                    s.focus = Some(i);
                }
            }
            _ => return,
        }
        if activate {
            self.on_nav(NavAction::Accept);
        } else {
            self.render();
        }
    }

    // ================================================================ startup animation

    /// Plays the startup animation (about 2.5 s), then shows the profile picker.
    pub fn start_splash(&mut self) {
        if self.config.startup_off {
            self.end_splash();
            return;
        }
        if crate::video::find(&self.paths.root).is_some() {
            // The window has to exist before the video view can attach to it: retry briefly.
            self.splash_started = Instant::now();
            self.splash_timer.start(slint::TimerMode::Repeated, Duration::from_millis(50), || {
                crate::with_app(|app| app.try_start_video());
            });
            return;
        }
        self.start_builtin_splash();
    }

    fn try_start_video(&mut self) {
        let Some(video) = crate::video::find(&self.paths.root) else { return self.start_builtin_splash() };
        if self.startup_video.is_none() {
            match crate::video::StartupWindow::new() {
                Ok(w) => self.startup_video = Some(w),
                Err(e) => {
                    tracing::warn!("startup video window: {e}");
                    return self.start_builtin_splash();
                }
            }
        }
        let cache = self.paths.root.join("cache");
        let attached = self
            .startup_video
            .as_mut()
            .map(|w| w.attach(&cache, &video, || post(|app| app.end_splash())))
            .unwrap_or(Ok(false));
        match attached {
            Ok(true) => {
                // Safety net in case the video never reports its end.
                self.splash_timer.start(slint::TimerMode::SingleShot, Duration::from_secs(60), || {
                    crate::with_app(|app| app.end_splash());
                });
            }
            Err(e) => {
                tracing::warn!("startup video: {e}");
                self.startup_video = None;
                self.start_builtin_splash();
            }
            Ok(false) if self.splash_started.elapsed() > Duration::from_secs(3) => {
                tracing::warn!("startup video: window never became ready");
                self.startup_video = None;
                self.start_builtin_splash();
            }
            Ok(false) => {}
        }
    }

    fn start_builtin_splash(&mut self) {
        crate::sounds::play(crate::sounds::Sound::Startup);
        self.splash_started = Instant::now();
        self.ui().set_splash_petals(self.splash_model.clone().into());
        self.splash_timer.start(slint::TimerMode::Repeated, Duration::from_millis(16), || {
            crate::with_app(|app| app.splash_tick());
        });
        self.splash_tick();
    }

    fn splash_tick(&mut self) {
        let t = self.splash_started.elapsed().as_secs_f32();
        if t >= crate::splash::DURATION {
            self.end_splash();
            return;
        }
        let f = crate::splash::frame(t);
        let ui = self.ui();
        ui.set_splash_outline(f.outline.into());
        ui.set_splash_outline_opacity(f.outline_opacity);
        ui.set_splash_sphere_radius(f.sphere_radius);
        ui.set_splash_sphere_opacity(f.sphere_opacity);
        ui.set_splash_emblem_radius(f.emblem_radius);
        ui.set_splash_emblem_opacity(f.emblem_opacity);
        ui.set_splash_wordmark_opacity(f.wordmark_opacity);
        ui.set_splash_fade(f.opacity);
        ui.set_splash_shadow_radius(f.shadow.0);
        ui.set_splash_shadow_opacity(f.shadow.1);
        let petals = f
            .petals
            .into_iter()
            .map(|p| crate::ui::SplashPetal { path: p.path.into(), shade: p.shade, opacity: p.opacity })
            .collect();
        sync(&self.splash_model, petals);
    }

    fn end_splash(&mut self) {
        if self.view != View::Splash || self.splash_ending {
            return;
        }
        self.splash_ending = true;
        self.splash_timer.stop();
        // Already black (animation off, or the built-in one ran to the end): go straight to the fade-in.
        let black = self.config.startup_off || (self.startup_video.is_none() && self.ui().get_splash_fade() < 0.05);
        self.splash_started = Instant::now();
        if black {
            self.splash_started -= Duration::from_secs_f32(INTRO_FADE_OUT);
        }
        if let Some(video) = &self.startup_video {
            video.fade_out(INTRO_FADE_OUT);
        }
        self.splash_timer.start(slint::TimerMode::Repeated, Duration::from_millis(16), || {
            crate::with_app(|app| app.intro_fade_tick());
        });
        self.intro_fade_tick();
    }

    fn intro_fade_tick(&mut self) {
        let t = self.splash_started.elapsed().as_secs_f32();
        if self.view == View::Splash {
            if self.startup_video.is_none() {
                self.ui().set_intro_black(ease_out(t / INTRO_FADE_OUT));
            }
            if t < INTRO_FADE_OUT {
                return;
            }
            self.ui().set_intro_black(1.0);
            self.view = View::Profiles;
            self.render();
            if self.startup_video.is_some() {
                // The dashboard is drawn (black) beneath the video's own window; close it once that frame is
                // up, then start the fade-in.
                self.ui().window().request_redraw();
                slint::Timer::single_shot(Duration::from_millis(80), || {
                    crate::with_app(|app| app.startup_video = None);
                });
                self.splash_started = Instant::now() + Duration::from_millis(100);
            } else {
                self.splash_started = Instant::now();
            }
            return;
        }
        let k = (t / INTRO_FADE_IN).min(1.0);
        self.ui().set_intro_black(1.0 - ease_out(k));
        if k >= 1.0 {
            self.splash_timer.stop();
        }
    }

    // ================================================================ profiles

    fn nav_profiles(&mut self, action: NavAction) {
        let count = self.profiles.len() + 1; // + "Add new"
        match action {
            NavAction::Left => self.profiles_focus = list_move(self.profiles_focus, count, -1),
            NavAction::Right => self.profiles_focus = list_move(self.profiles_focus, count, 1),
            NavAction::Accept if self.profiles_focus == self.profiles.len() => {
                self.open_keyboard("Name your profile", "", 24, OskPurpose::NewProfile);
            }
            NavAction::Accept => self.sign_in(self.profiles_focus),
            _ => {}
        }
    }

    fn sign_in(&mut self, index: usize) {
        let Some(profile) = self.profiles.get(index).cloned() else { return };
        crate::sounds::play(crate::sounds::Sound::SignIn);
        self.config.last_profile = Some(profile.id);
        self.save_config();

        self.current = Some(profile);
        self.home_section = 0;
        self.hero_focus = 0;
        self.lib_focus = 0;
        self.lib_zone = LibZone::Sidebar;
        self.go(View::Home);
        self.update_libraries();
        self.start_social();
    }

    fn switch_profile(&mut self) {
        self.profiles = self.store.list().unwrap_or_default();
        self.profiles_focus =
            self.profile_id().and_then(|id| self.profiles.iter().position(|p| p.id == id)).unwrap_or(0);
        self.current = None;
        self.modal = None;
        self.view = View::Profiles;
        self.stop_social();
    }

    // ================================================================ navigation between screens

    fn go(&mut self, view: View) {
        if matches!(view, View::Settings | View::Search) && matches!(self.view, View::Home | View::Library) {
            self.back_to = self.view;
        }
        self.view = view;
        self.reload();
    }

    /// Re-reads library data for the current screen.
    fn reload(&mut self) {
        match self.view {
            View::Home => self.reload_home(),
            View::Library => self.reload_library(),
            View::Search => self.update_search(),
            _ => {}
        }
    }

    fn toggle_guide(&mut self) {
        self.guide_chat = None;
        if self.ingame.is_some() || matches!(self.modal, Some(ModalState::Guide { .. })) {
            self.close_guide();
        } else if self.current.is_some() && self.view != View::Profiles {
            if (self.running.is_some() || self.desktop) && !cs_platform::window::own_window_is_foreground() {
                self.open_ingame_guide();
            }
            self.modal = Some(ModalState::Guide { tab: 0, focus: 0 });
        }
    }

    /// Captures the game's screen (the Guide's backdrop and the screenshot), then comes to the front.
    fn open_ingame_guide(&mut self) {
        let capture = match cs_platform::capture::primary_screen() {
            Ok(c) => Some(Arc::new(c)),
            Err(e) => {
                tracing::warn!("screen capture failed: {e}");
                None
            }
        };
        let backdrop = capture.as_ref().map(|c| {
            slint::Image::from_rgba8(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                &c.rgba, c.width, c.height,
            ))
        });
        self.ingame = Some(InGame { capture, backdrop, shown: false });
        // The controller navigates the Guide, not the pointer.
        cs_input::set_mouse_mode(false);
        cs_platform::window::bring_own_window_to_front();
    }

    /// Closes the Guide; over a game, goes back to the game.
    fn close_guide(&mut self) {
        self.modal = None;
        self.guide_chat = None;
        if self.ingame.take().is_some() {
            self.resume_game();
            if self.desktop {
                cs_input::set_mouse_mode(true);
            }
        }
    }

    /// Leaves the in-game Guide for the dashboard (the game keeps running in the background).
    fn leave_ingame(&mut self) {
        self.ingame = None;
        self.leave_desktop();
    }

    fn resume_game(&mut self) {
        let pids = self.game_pids();
        if !cs_platform::window::focus_process(&pids) {
            cs_platform::window::minimize_own_window();
        }
    }

    fn game_pids(&self) -> Vec<u32> {
        self.running.as_ref().map(|r| r.handle.pids(&SystemProcesses)).unwrap_or_default()
    }

    fn running_entry(&self) -> Option<GameEntry> {
        self.running.as_ref().and_then(|r| self.entry_by_id(&r.game_id))
    }

    // ================================================================ home

    fn reload_home(&mut self) {
        let Some(p) = self.profile_id() else { return };
        let recent = self.library.recent(p, RECENT_COUNT).unwrap_or_else(log_db_err);
        let pinned = self.library.pinned(p).unwrap_or_else(log_db_err);
        let all = self.library.list(p, false).unwrap_or_else(log_db_err);

        // Hero grid (positions measured from the Xbox dashboard).
        let mut hero = Vec::new();
        let big = recent.first().or(all.first()).cloned();
        let big_id = big.as_ref().map(|e| e.game.id.clone());
        hero.push(HeroTile {
            rect: Rect::new(99.0, 258.0, 390.0, 387.0),
            item: big.map_or(HeroItem::Shortcut(Shortcut::AddGame), HeroItem::Game),
            wide: false,
            caption: true,
        });
        // The row scrolls: games you've played (then pinned ones), with the shortcuts after them.
        let mut games: Vec<GameEntry> = Vec::new();
        for e in recent.iter().chain(pinned.iter()) {
            if Some(&e.game.id) != big_id.as_ref() && !games.iter().any(|g| g.game.id == e.game.id) {
                games.push(e.clone());
            }
        }
        let mut squares: Vec<HeroItem> = games.into_iter().take(10).map(HeroItem::Game).collect();
        squares.extend(Shortcut::ALL.into_iter().map(HeroItem::Shortcut));
        for (i, item) in squares.into_iter().enumerate() {
            hero.push(HeroTile {
                rect: Rect::new(536.0 + 216.0 * i as f32, 440.0, 200.0, 200.0),
                item,
                wide: false,
                caption: false,
            });
        }
        hero.push(HeroTile {
            rect: Rect::new(99.0, 692.0, 386.0, 216.0),
            item: HeroItem::Library,
            wide: false,
            caption: true,
        });
        for (i, e) in recent.iter().skip(1).take(3).enumerate() {
            hero.push(HeroTile {
                rect: Rect::new(536.0 + 431.0 * i as f32, 692.0, 416.0, 216.0),
                item: HeroItem::Game(e.clone()),
                wide: true,
                caption: true,
            });
        }
        self.hero = hero;
        self.hero_focus = self.hero_focus.min(self.hero.len() - 1);

        let mut sections = Vec::new();
        // Store and Game Pass rows, like the dashboard's lower sections.
        for (title, browse, feed) in
            [("Store", "store:browse", &self.store_feed), ("Game Pass", "gamepass:browse", &self.gamepass_feed)]
        {
            let mut entries = vec![web_entry(browse, &format!("Browse {title}"), None, "")];
            entries.extend(feed.iter().cloned());
            sections.push(Section { title: title.into(), entries });
        }
        if !pinned.is_empty() {
            sections.push(Section { title: "Pinned".into(), entries: pinned });
        }
        for provider in Provider::ALL {
            let entries: Vec<GameEntry> =
                all.iter().filter(|e| e.game.provider == provider && e.installed).cloned().collect();
            if !entries.is_empty() {
                sections.push(Section { title: provider.display_name().into(), entries });
            }
        }
        self.sections = sections;
        if self.home_section > self.sections.len() {
            self.home_section = self.sections.len();
        }
        if self.home_section > 0 {
            let len = self.sections[self.home_section - 1].entries.len();
            self.section_focus = self.section_focus.min(len.saturating_sub(1));
        }
    }

    fn nav_home(&mut self, action: NavAction) {
        let dir = matches!(action, NavAction::Up | NavAction::Down | NavAction::Left | NavAction::Right);
        if self.home_section == 0 {
            if dir {
                let rects: Vec<Rect> = self.hero.iter().map(|t| t.rect).collect();
                match spatial_move(&rects, self.hero_focus, action) {
                    Some(f) => self.hero_focus = f,
                    None if action == NavAction::Down && !self.sections.is_empty() => {
                        self.home_section = 1;
                        self.section_focus = 0;
                    }
                    None => {}
                }
                return;
            }
        } else if dir {
            let len = self.sections[self.home_section - 1].entries.len();
            match action {
                NavAction::Left => self.section_focus = list_move(self.section_focus, len, -1),
                NavAction::Right => self.section_focus = list_move(self.section_focus, len, 1),
                NavAction::Up => {
                    self.home_section -= 1;
                    self.section_focus = 0;
                }
                NavAction::Down if self.home_section < self.sections.len() => {
                    self.home_section += 1;
                    self.section_focus = 0;
                }
                _ => {}
            }
            return;
        }
        match action {
            NavAction::Accept => match self.home_item() {
                Some(HeroItem::Game(e)) => self.play(&e),
                Some(HeroItem::Shortcut(s)) => self.shortcut(s),
                Some(HeroItem::Library) => self.open_library(),
                None => {}
            },
            NavAction::Back if self.home_section > 0 => self.home_section = 0,
            NavAction::X => {
                if let Some(HeroItem::Game(e)) = self.home_item().filter(|_| !self.on_web_item()) {
                    self.toggle_pin(&e.game.id);
                }
            }
            NavAction::Menu => {
                if let Some(HeroItem::Game(e)) = self.home_item().filter(|_| !self.on_web_item()) {
                    self.open_game_menu(&e);
                }
            }
            NavAction::Y => self.open_search(),
            NavAction::View => self.open_settings(0),
            NavAction::NextTab => self.open_library(),
            _ => {}
        }
    }

    fn home_item(&self) -> Option<HeroItem> {
        if self.home_section == 0 {
            self.hero.get(self.hero_focus).map(|t| t.item.clone())
        } else {
            self.sections[self.home_section - 1].entries.get(self.section_focus).cloned().map(HeroItem::Game)
        }
    }

    fn on_web_item(&self) -> bool {
        matches!(self.home_item(), Some(HeroItem::Game(e)) if web_kind(&e.game.id).is_some())
    }

    /// Fetches the Store and Game Pass rows in the background.
    pub fn load_feeds(&mut self) {
        std::thread::spawn(|| {
            let to_entries = |kind: &str, items: Result<Vec<crate::feeds::FeedItem>, String>| {
                items
                    .inspect_err(|e| tracing::warn!("{kind} feed: {e}"))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|i| web_entry(&format!("{kind}:{}", i.id), &i.name, Some(i.image), &i.page))
                    .collect::<Vec<_>>()
            };
            let store = to_entries("store", crate::feeds::steam_top_sellers());
            let gamepass = to_entries("gamepass", crate::feeds::game_pass());
            post(move |app| {
                app.store_feed = store;
                app.gamepass_feed = gamepass;
                app.reload();
                app.render();
            });
        });
    }

    fn shortcut(&mut self, s: Shortcut) {
        match s {
            Shortcut::Settings => self.open_settings(0),
            Shortcut::AddGame => self.open_files("Choose the game's .exe", Filter::Executables, FilePurpose::AddGame),
            Shortcut::Search => self.open_search(),
            Shortcut::SwitchProfile => self.switch_profile(),
            Shortcut::Power => self.open_power_menu(),
        }
    }

    // ================================================================ my games & apps

    fn open_library(&mut self) {
        self.modal = None;
        self.lib_zone = LibZone::Sidebar;
        self.go(View::Library);
    }

    /// Sidebar entries for the signed-in profile.
    fn lib_entries(&self) -> Vec<LibEntry> {
        let mut v = vec![LibEntry::All, LibEntry::Installed];
        let present: Vec<Provider> = self
            .profile_id()
            .map(|p| self.library.list(p, true).unwrap_or_default())
            .unwrap_or_default()
            .iter()
            .map(|e| e.game.provider)
            .collect();
        v.extend(
            Provider::ALL
                .into_iter()
                .filter(|pr| *pr != Provider::Custom && present.contains(pr))
                .map(LibEntry::Provider),
        );
        v.extend([LibEntry::Provider(Provider::Custom), LibEntry::Hidden, LibEntry::ManageAddons, LibEntry::Update]);
        v
    }

    fn lib_entry(&self) -> LibEntry {
        self.lib_entries().get(self.lib_category).copied().unwrap_or(LibEntry::All)
    }

    fn reload_library(&mut self) {
        let Some(p) = self.profile_id() else { return };
        let everything = self.library.list(p, true).unwrap_or_else(log_db_err);
        let entries = self.lib_entries();
        self.lib_category = self.lib_category.min(entries.len() - 1);
        let show_hidden = self.show_hidden;
        let matches = |entry: LibEntry, e: &GameEntry| match entry {
            LibEntry::Provider(pr) => (show_hidden || !e.stats.hidden) && e.game.provider == pr,
            LibEntry::Hidden => e.stats.hidden,
            LibEntry::Installed => (show_hidden || !e.stats.hidden) && e.installed,
            _ => show_hidden || !e.stats.hidden,
        };
        self.lib_counts = entries
            .iter()
            .map(|&entry| match entry {
                LibEntry::ManageAddons | LibEntry::Update => 0,
                _ => everything.iter().filter(|e| matches(entry, e)).count(),
            })
            .collect();
        let entry = entries[self.lib_category];
        let mut games: Vec<GameEntry> = everything.into_iter().filter(|e| matches(entry, e)).collect();
        match self.lib_sort {
            Sort::Name => {} // already sorted by name
            Sort::Recent => games.sort_by(|a, b| b.stats.last_played.cmp(&a.stats.last_played)),
            Sort::Playtime => games.sort_by(|a, b| b.stats.playtime_secs.cmp(&a.stats.playtime_secs)),
        }
        self.lib_games = games;
        self.lib_focus = self.lib_focus.min(self.lib_games.len().saturating_sub(1));
    }

    fn select_category(&mut self, category: usize) {
        if category < self.lib_entries().len() && category != self.lib_category {
            self.lib_category = category;
            self.lib_focus = 0;
            self.reload_library();
        }
    }

    /// Grid origin x and column count for the current sidebar state.
    fn lib_layout(&self) -> (f32, usize) {
        let origin = if self.lib_zone == LibZone::Sidebar { 549.0 } else { 146.0 };
        let width = self.ui().get_design_width().max(960.0);
        (origin, (((width - origin - 100.0 + 16.0) / 272.0).floor() as usize).max(1))
    }

    fn nav_library(&mut self, action: NavAction) {
        let len = self.lib_games.len();
        match (self.lib_zone, action) {
            (LibZone::Sidebar, NavAction::Up) => self.select_category(self.lib_category.saturating_sub(1)),
            (LibZone::Sidebar, NavAction::Down) => self.select_category(self.lib_category + 1),
            (LibZone::Sidebar, NavAction::Accept) if self.lib_entry() == LibEntry::ManageAddons => self.open_playnite(),
            (LibZone::Sidebar, NavAction::Accept) if self.lib_entry() == LibEntry::Update => self.update_libraries(),
            (LibZone::Sidebar, NavAction::Right | NavAction::Accept) => {
                self.lib_zone = if len > 0 { LibZone::Grid } else { LibZone::Controls(1) };
            }
            (LibZone::Sidebar, NavAction::Back) => self.go(View::Home),
            (LibZone::Controls(c), NavAction::Left) if c > 0 => self.lib_zone = LibZone::Controls(c - 1),
            (LibZone::Controls(_), NavAction::Left | NavAction::Back) => self.lib_zone = LibZone::Sidebar,
            (LibZone::Controls(c), NavAction::Right) => self.lib_zone = LibZone::Controls((c + 1).min(1)),
            (LibZone::Controls(_), NavAction::Down) if len > 0 => self.lib_zone = LibZone::Grid,
            (LibZone::Controls(0), NavAction::Accept) => self.open_sort_menu(),
            (LibZone::Controls(_), NavAction::Accept) => {
                self.open_files("Choose the game's .exe", Filter::Executables, FilePurpose::AddGame)
            }
            (LibZone::Grid, NavAction::Up | NavAction::Down | NavAction::Left | NavAction::Right) => {
                let (_, cols) = self.lib_layout();
                match crate::nav::grid_move(self.lib_focus, len, cols, action) {
                    Some(f) => self.lib_focus = f,
                    None if action == NavAction::Left => self.lib_zone = LibZone::Sidebar,
                    None if action == NavAction::Up => self.lib_zone = LibZone::Controls(0),
                    None => {}
                }
            }
            (LibZone::Grid, NavAction::Back) => self.lib_zone = LibZone::Sidebar,
            (LibZone::Grid, NavAction::Accept) => {
                if let Some(e) = self.lib_games.get(self.lib_focus).cloned() {
                    self.play(&e);
                }
            }
            (LibZone::Grid, NavAction::X) => {
                if let Some(id) = self.lib_games.get(self.lib_focus).map(|e| e.game.id.clone()) {
                    self.toggle_pin(&id);
                }
            }
            (LibZone::Grid, NavAction::Menu) => {
                if let Some(e) = self.lib_games.get(self.lib_focus).cloned() {
                    self.open_game_menu(&e);
                }
            }
            (_, NavAction::Y) => self.open_sort_menu(),
            (_, NavAction::View) => {
                self.open_files("Choose the game's .exe", Filter::Executables, FilePurpose::AddGame)
            }
            _ => {}
        }
    }

    fn open_sort_menu(&mut self) {
        let entries = Sort::ALL
            .iter()
            .map(|&s| entry(if s == self.lib_sort { "\u{E73E}" } else { "" }, s.label(), MenuAction::SortBy(s)))
            .collect();
        self.open_menu("Sort by", "", entries);
    }

    // ================================================================ settings

    fn open_settings(&mut self, category: usize) {
        self.modal = None;
        self.settings_category = category;
        self.settings_in_content = false;
        self.settings_focus = 0;
        self.go(View::Settings);
    }

    fn setting_rects(category: usize) -> Vec<Rect> {
        (0..settings_for(category).len())
            .map(|i| Rect::new(552.0 + (i % 2) as f32 * 453.0, 201.0 + (i / 2) as f32 * 148.0, 438.0, 132.0))
            .collect()
    }

    fn nav_settings(&mut self, action: NavAction) {
        if !self.settings_in_content {
            match action {
                NavAction::Up => self.settings_category = self.settings_category.saturating_sub(1),
                NavAction::Down => {
                    self.settings_category = (self.settings_category + 1).min(SETTINGS_CATEGORIES.len() - 1)
                }
                NavAction::Right | NavAction::Accept => {
                    self.settings_in_content = true;
                    self.settings_focus = 0;
                }
                NavAction::Back => self.go(self.back_to),
                _ => {}
            }
            return;
        }
        match action {
            NavAction::Up | NavAction::Down | NavAction::Left | NavAction::Right => {
                let rects = Self::setting_rects(self.settings_category);
                match spatial_move(&rects, self.settings_focus, action) {
                    Some(f) => self.settings_focus = f,
                    None if action == NavAction::Left => self.settings_in_content = false,
                    None => {}
                }
            }
            NavAction::Back => self.settings_in_content = false,
            NavAction::Accept => {
                if let Some(&s) = settings_for(self.settings_category).get(self.settings_focus) {
                    self.activate_setting(s);
                }
            }
            _ => {}
        }
    }

    fn activate_setting(&mut self, setting: Setting) {
        let Some(profile) = self.current.clone() else { return };
        match setting {
            Setting::Name => self.open_keyboard("Profile name", &profile.name, 24, OskPurpose::RenameProfile),
            Setting::Avatar => self.open_picker(PickerKind::Avatar { onboarding: false }),
            Setting::Accent => self.open_picker(PickerKind::Accent { onboarding: false }),
            Setting::Background => self.open_picker(PickerKind::Background),
            Setting::SwitchProfile => self.switch_profile(),
            Setting::DeleteProfile => self.open_menu(
                &format!("Delete {}?", profile.name),
                "This removes the profile, its playtime and its pins. Games stay installed.",
                vec![
                    entry("\u{E74D}", "Delete profile", MenuAction::ConfirmDeleteProfile),
                    entry("\u{E711}", "Cancel", MenuAction::Close),
                ],
            ),
            Setting::LinkDiscord => self.start_sign_in(Service::Discord),
            Setting::AppTitle => {
                let current = self.config.app_title.clone().unwrap_or_default();
                self.open_keyboard("App name (leave empty for Win2Console)", &current, 40, OskPurpose::AppTitle);
            }
            Setting::Updates => {
                let mut entries = vec![entry("\u{E72C}", "Check for updates", MenuAction::CheckUpdates)];
                if matches!(self.update_state, updater::State::Available(_) | updater::State::Ready(_)) {
                    entries.push(entry(
                        "\u{E896}",
                        "Install update now (restarts Win2Console)",
                        MenuAction::InstallUpdate,
                    ));
                }
                let auto = if self.config.manual_updates { "Automatic updates: Off" } else { "Automatic updates: On" };
                entries.push(entry("\u{E895}", auto, MenuAction::ToggleAutoUpdate));
                let message = format!(
                    "Win2Console {}. {}",
                    env!("CARGO_PKG_VERSION"),
                    if self.config.manual_updates {
                        "Updates are only installed when you choose to."
                    } else {
                        "Updates download by themselves and install when you close Win2Console."
                    }
                );
                self.open_menu("Updates", &message, entries);
            }
            Setting::MenuSize => {
                let current = self.config.ui_scale.unwrap_or(100);
                let entries = [80, 90, 100, 110, 125]
                    .into_iter()
                    .map(|p| {
                        let label = if p == current { format!("{p}%  (current)") } else { format!("{p}%") };
                        entry("\u{E740}", &label, MenuAction::UiScale(p))
                    })
                    .collect();
                self.open_menu("Menu size", "Makes everything on screen bigger or smaller.", entries);
            }
            Setting::StartupAnimation => self.open_menu(
                "Startup animation",
                "Plays when Win2Console starts. Any button skips it.",
                vec![
                    entry("\u{E790}", "Built-in animation", MenuAction::Startup(StartupChoice::BuiltIn)),
                    entry("\u{E714}", "Choose a video (.mp4 or .webm)", MenuAction::Startup(StartupChoice::Video)),
                    entry("\u{E711}", "Off", MenuAction::Startup(StartupChoice::Off)),
                ],
            ),
            Setting::AppIcon => self.open_menu(
                "App icon",
                "Shown in the taskbar and window title bar.",
                vec![
                    entry("\u{EB9F}", "Choose an image", MenuAction::ChooseAppIcon),
                    entry("\u{E72C}", "Use the default icon", MenuAction::ResetAppIcon),
                ],
            ),
            Setting::Music => {
                self.config.music_off = !self.config.music_off;
                self.save_config();
            }
            Setting::SteamAccount => {
                let mut entries: Vec<MenuEntry> = cs_library::steam::users()
                    .into_iter()
                    .map(|u| {
                        let label = format!("{} ({})", u.persona, u.account);
                        entry("\u{E77B}", &label, MenuAction::SetSteamAccount(Some((u.account, u.persona))))
                    })
                    .collect();
                entries.push(entry("\u{E711}", "Don't switch accounts", MenuAction::SetSteamAccount(None)));
                self.open_menu(
                    "Steam account",
                    "Steam games on this profile start signed in to this account. Accounts appear here after \
                     signing in to Steam once with \"Remember me\".",
                    entries,
                );
            }

            Setting::ShowHidden => self.show_hidden = !self.show_hidden,
            Setting::Rescan => self.update_libraries(),
            Setting::AddGame => self.open_files("Choose the game's .exe", Filter::Executables, FilePurpose::AddGame),
            Setting::StartWithWindows => self.toggle_start_with_windows(),
            Setting::About => self.view = View::About,
            Setting::Sleep => self.power(cs_platform::power::sleep()),
            Setting::Restart => self.power(cs_platform::power::restart()),
            Setting::ShutDown => self.power(cs_platform::power::shut_down()),
            Setting::ExitToDesktop => self.exit_to_desktop(),
            Setting::Quit => self.quit(),
        }
    }

    fn setting_tile(&self, setting: Setting, rect: Rect) -> SettingTile {
        let p = self.current.as_ref();
        let (glyph, label, value, toggle) = match setting {
            Setting::Name => ("\u{E8AC}", "Profile name", p.map(|p| p.name.clone()).unwrap_or_default(), -1),
            Setting::Avatar => ("\u{E77B}", "Profile picture", String::new(), -1),
            Setting::Accent => ("\u{E790}", "My color", p.map(|p| p.accent.clone()).unwrap_or_default(), -1),
            Setting::Background => (
                "\u{EB9F}",
                "My background",
                match p.map(|p| p.background) {
                    Some(Background::Solid) => "Solid color",
                    Some(Background::Custom) => "Custom image",
                    _ => "Default",
                }
                .to_string(),
                -1,
            ),
            Setting::SwitchProfile => ("\u{E748}", "Switch profile", String::new(), -1),
            Setting::DeleteProfile => ("\u{E74D}", "Delete this profile", String::new(), -1),
            Setting::LinkDiscord => (
                "\u{E716}",
                "Discord",
                p.and_then(|p| p.accounts.iter().find(|a| a.service == Service::Discord))
                    .map_or("Not linked".to_string(), |a| format!("Linked as {}", a.display_name)),
                -1,
            ),
            Setting::SteamAccount => (
                "\u{E7FC}",
                "Steam account",
                p.and_then(|p| p.accounts.iter().find(|a| a.service == Service::Steam))
                    .map_or("Any account".to_string(), |a| a.display_name.clone()),
                -1,
            ),
            Setting::ShowHidden => ("\u{ED1A}", "Show hidden games", String::new(), self.show_hidden as i32),
            Setting::Rescan => (
                "\u{E72C}",
                "Update game library",
                if self.scanning { "Updating…".into() } else { self.scan_summary.clone() },
                -1,
            ),
            Setting::AddGame => ("\u{E710}", "Add a game", String::new(), -1),
            Setting::AppTitle => {
                ("\u{E8AC}", "App name", self.config.app_title.clone().unwrap_or_else(|| "Win2Console".into()), -1)
            }
            Setting::AppIcon => ("\u{E91B}", "App icon", String::new(), -1),
            Setting::Updates => ("\u{E895}", "Updates", self.update_state.label(), -1),
            Setting::MenuSize => ("\u{E740}", "Menu size", format!("{}%", self.config.ui_scale.unwrap_or(100)), -1),
            Setting::StartupAnimation => (
                "\u{E714}",
                "Startup animation",
                if self.config.startup_off {
                    "Off".to_string()
                } else {
                    crate::video::find(&self.paths.root).map_or("Built-in".to_string(), |_| "Custom video".to_string())
                },
                -1,
            ),
            Setting::Music => ("\u{E8D6}", "Background music", String::new(), !self.config.music_off as i32),
            Setting::StartWithWindows => {
                ("\u{E7B5}", "Start with Windows", String::new(), cs_platform::startup::enabled() as i32)
            }
            Setting::About => ("\u{E946}", "About Win2Console", String::new(), -1),
            Setting::Sleep => ("\u{E708}", "Sleep", String::new(), -1),
            Setting::Restart => ("\u{E777}", "Restart", String::new(), -1),
            Setting::ShutDown => ("\u{E7E8}", "Shut down", String::new(), -1),
            Setting::ExitToDesktop => ("\u{E89F}", "Exit to desktop", String::new(), -1),
            Setting::Quit => ("\u{E711}", "Close Win2Console", String::new(), -1),
        };
        SettingTile {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
            glyph: glyph.into(),
            label: label.into(),
            value: value.into(),
            toggle,
            corner: setting == Setting::Accent,
            chevron: matches!(setting, Setting::Background | Setting::Avatar),
        }
    }

    /// Re-registers the startup task with the current exe path (and moves old `Run` key users over).
    pub fn refresh_start_with_windows(&self) {
        if cs_platform::startup::enabled()
            && let Ok(exe) = std::env::current_exe()
        {
            let _ = cs_platform::startup::set(Some(&exe));
        }
    }

    fn toggle_start_with_windows(&mut self) {
        let enable = !cs_platform::startup::enabled();
        let exe = std::env::current_exe().ok();
        match cs_platform::startup::set(if enable { exe.as_deref() } else { None }) {
            Ok(()) => {
                self.config.start_with_windows = enable;
                self.save_config();
            }
            Err(e) => self.toast(format!("Couldn't change the startup setting: {e}")),
        }
    }

    // ================================================================ files

    fn open_files(&mut self, title: &str, filter: Filter, purpose: FilePurpose) {
        let start = match filter {
            Filter::Executables => None,
            Filter::Images => std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Pictures")),
            Filter::Videos => std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Videos")),
        };
        let ret = if self.view == View::Files { View::Home } else { self.view };
        self.files = Some((FileBrowser::new(title, filter, start), purpose, ret));
        self.modal = None;
        self.view = View::Files;
    }

    fn nav_files(&mut self, action: NavAction) {
        let Some((fb, _, _)) = &mut self.files else { return };
        match fb.handle(action) {
            FbOutcome::Continue => {}
            FbOutcome::Cancel => {
                let (_, _, ret) = self.files.take().expect("checked above");
                self.go(ret);
            }
            FbOutcome::Picked(path) => {
                let (_, purpose, ret) = self.files.take().expect("checked above");
                self.go(ret);
                self.file_picked(path, purpose);
            }
        }
    }

    fn file_picked(&mut self, path: PathBuf, purpose: FilePurpose) {
        match purpose {
            FilePurpose::AddGame => {
                let name = cs_library::custom::default_name(&path);
                self.open_keyboard("Name this game", &name, 48, OskPurpose::NewGameName { exe: path });
            }
            FilePurpose::Art { game_id, slot } => self.set_art(&game_id, slot, &path),
            FilePurpose::Avatar => self.set_custom_avatar(&path),
            FilePurpose::Background => self.set_custom_background(&path),
            FilePurpose::StartupVideo => {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_ascii_lowercase();
                for f in ["startup.mp4", "startup.webm"] {
                    let _ = std::fs::remove_file(self.paths.root.join(f));
                }
                match std::fs::copy(&path, self.paths.root.join(format!("startup.{ext}"))) {
                    Ok(_) => {
                        self.config.startup_off = false;
                        self.save_config();
                        self.toast("Startup video set");
                    }
                    Err(e) => self.toast(format!("Couldn't use that video: {e}")),
                }
            }
            FilePurpose::AppIcon => {
                let result = images::png_fit(&path, (256, 256))
                    .map_err(|e| e.to_string())
                    .and_then(|png| std::fs::write(self.paths.root.join(APP_ICON), png).map_err(|e| e.to_string()));
                match result {
                    Ok(()) => {
                        self.apply_branding();
                        self.toast("App icon changed");
                    }
                    Err(e) => {
                        tracing::warn!("app icon {}: {e}", path.display());
                        self.toast(format!("Couldn't use that image: {e}"));
                    }
                }
            }
        }
    }

    // ================================================================ search

    fn open_search(&mut self) {
        self.modal = None;
        let ret = if self.view == View::Search { View::Home } else { self.view };
        let mut osk = Osk::new("Search", "", 40);
        osk.placeholder = "Search your games".into();
        self.search = Some(SearchState { osk, results: Vec::new(), focus: None, ret });
        self.go(View::Search);
    }

    fn update_search(&mut self) {
        let Some(p) = self.profile_id() else { return };
        let Some(s) = &mut self.search else { return };
        let query = s.osk.text().trim().to_lowercase();
        s.results = if query.is_empty() {
            Vec::new()
        } else {
            let mut hits: Vec<GameEntry> = self
                .library
                .list(p, false)
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.game.name.to_lowercase().contains(&query))
                .collect();
            // Prefix matches first, then alphabetical (already sorted).
            hits.sort_by_key(|e| !e.game.name.to_lowercase().starts_with(&query));
            hits.truncate(8);
            hits
        };
        if let Some(f) = s.focus {
            s.focus = (!s.results.is_empty()).then(|| f.min(s.results.len() - 1));
        }
    }

    fn nav_search(&mut self, action: NavAction) {
        let Some(s) = &mut self.search else { return };
        match s.focus {
            None => {
                if action == NavAction::Up && s.osk.focus_on_top_row() && !s.results.is_empty() {
                    s.focus = Some(0);
                    return;
                }
                let outcome = s.osk.handle(action);
                self.osk_outcome(outcome);
            }
            Some(f) => match action {
                NavAction::Left => s.focus = Some(f.saturating_sub(1)),
                NavAction::Right => s.focus = Some((f + 1).min(s.results.len() - 1)),
                NavAction::Down | NavAction::Back => s.focus = None,
                NavAction::Accept => {
                    let e = s.results[f].clone();
                    self.play(&e);
                }
                NavAction::Menu => {
                    let e = s.results[f].clone();
                    self.open_game_menu(&e);
                }
                _ => {}
            },
        }
    }

    // ================================================================ sign-in

    fn start_sign_in(&mut self, service: Service) {
        let Some(profile) = self.profile_id() else { return };
        let pkce = Pkce::new();
        let app_id = self.config.discord_app_id.clone().unwrap_or_else(|| DEFAULT_DISCORD_APP_ID.to_string());
        let url = match (service, Some(&app_id)) {
            (Service::Discord, Some(app_id)) => format!(
                "https://discord.com/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
                crate::weblogin::encode(app_id),
                crate::weblogin::encode(DISCORD_REDIRECT),
                crate::weblogin::encode(&discord_scopes()),
                pkce.challenge,
                pkce.state
            ),
            // Without an Application ID only Discord's own login page can be shown.
            (Service::Discord, None) => "https://discord.com/login".to_string(),
            _ => return,
        };
        let _ = service;
        self.open_web(profile, SignInTarget::Discord, &url, DISCORD_REDIRECT, pkce);
    }

    /// Store / Game Pass in the controller browser.
    fn open_browse(&mut self, title: &'static str, glyph: &'static str, url: &str) {
        let Some(profile) = self.profile_id() else { return };
        self.open_web(profile, SignInTarget::Browse { title, glyph }, url, "", Pkce::new());
    }

    fn open_web(&mut self, profile: uuid::Uuid, target: SignInTarget, url: &str, redirect: &str, pkce: Pkce) {
        let data_dir = self.store.profile_dir(profile).join("webview");
        let accent = self.current.as_ref().map_or(cs_core::model::DEFAULT_ACCENT.to_string(), |p| p.accent.clone());
        let ui = self.ui();
        let opened = WebLogin::open(ui.window(), data_dir, url, redirect, "", &accent, |ev| {
            crate::with_app(|app| app.web_event(ev));
        });
        match opened {
            Ok(web) => {
                let ret = if self.view == View::SignIn { View::Home } else { self.view };
                self.modal = None;
                self.signin = Some(SignInState {
                    target,
                    web,
                    pkce,
                    zone: SignInZone::Page,
                    field: FocusInfo::default(),
                    osk: Osk::new("", "", 256),
                    ret,
                });
                self.view = View::SignIn;
            }
            Err(e) => self.toast(format!("Couldn't open the page: {e}")),
        }
    }

    fn close_sign_in(&mut self) {
        if let Some(si) = self.signin.take() {
            si.web.focus_parent();
            self.go(si.ret);
        }
    }

    fn nav_signin(&mut self, action: NavAction) {
        let Some(si) = &mut self.signin else { return };
        if si.zone == SignInZone::Keyboard {
            let outcome = si.osk.handle(action);
            self.osk_outcome(outcome);
            return;
        }
        match action {
            NavAction::Up => si.web.nav("up"),
            NavAction::Down => si.web.nav("down"),
            NavAction::Left => si.web.nav("left"),
            NavAction::Right => si.web.nav("right"),
            NavAction::Accept if si.field.text => {
                let mut osk = Osk::new(si.field.label.clone(), &si.field.value, 256);
                osk.placeholder = si.field.label.clone();
                si.osk = osk;
                si.zone = SignInZone::Keyboard;
            }
            NavAction::Accept => si.web.activate(),
            NavAction::Menu => si.web.submit(),
            NavAction::View => si.web.back(),
            NavAction::Back => self.close_sign_in(),
            _ => {}
        }
    }

    fn web_event(&mut self, ev: WebEvent) {
        let Some(si) = &mut self.signin else { return };
        match ev {
            WebEvent::Focus(info) => si.field = info,
            WebEvent::Page { .. } => {}
            WebEvent::Redirect(params) => {
                let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
                if get("state").as_deref() != Some(si.pkce.state.as_str()) {
                    tracing::warn!("sign-in redirect with unexpected state ignored");
                    return;
                }
                let service = Service::Discord;
                match (get("code"), get("error")) {
                    (Some(code), _) => {
                        let verifier = si.pkce.verifier.clone();
                        self.close_sign_in();
                        self.authorization_received(service, code, verifier);
                    }
                    (None, err) => {
                        self.close_sign_in();
                        self.toast(format!(
                            "{} sign-in was cancelled{}",
                            service.display_name(),
                            err.map(|e| format!(" ({e})")).unwrap_or_default()
                        ));
                    }
                }
            }
        }
        self.render();
    }

    /// An OAuth code arrived. The Discord Social SDK exchanges it for tokens (added with the SDK).
    fn authorization_received(&mut self, service: Service, code: String, verifier: String) {
        tracing::info!("{} authorization received", service.display_name());
        match self.new_discord_backend() {
            Some(mut backend) => {
                backend.link(&code, &verifier, DISCORD_REDIRECT);
                self.social = Some(Box::new(backend));
                self.social_status = social::Status::Connecting;
                self.toast("Connecting to Discord…");
            }
            None => self.toast("This build of Win2Console doesn't include Discord"),
        }
    }

    fn render_signin(&mut self, ui: &AppWindow) {
        let Some(si) = &self.signin else { return };
        let (name, glyph) = match si.target {
            SignInTarget::Discord => ("Sign in to Discord", "\u{E716}"),
            SignInTarget::Browse { title, glyph } => (title, glyph),
        };
        ui.set_signin_title(name.into());
        ui.set_signin_glyph(glyph.into());
        ui.set_signin_status(
            match si.zone {
                SignInZone::Keyboard => "Typing",
                SignInZone::Page => "Use the D-pad to move between fields",
            }
            .into(),
        );
        let typing = si.zone == SignInZone::Keyboard;
        ui.set_signin_typing(typing);
        ui.set_signin_field(
            if typing && !si.field.label.is_empty() { si.field.label.clone() } else { String::new() }.into(),
        );
        // Place the web page over the frame drawn by the UI (design pixels -> logical pixels).
        let scale = ui.get_ui_scale();
        let width = ui.get_design_width();
        let height = ui.get_design_height();
        si.web.set_bounds(102.0 * scale, 149.0 * scale, (width - 204.0) * scale, (height - 537.0) * scale);
        let keys = key_views(&si.osk);
        let focus = si.osk.focus;
        sync(&self.models.kb_keys, keys);
        ui.set_kb_focus(focus as i32);
    }

    // ================================================================ modals

    fn nav_modal(&mut self, action: NavAction) {
        if matches!(self.modal, Some(ModalState::Guide { .. })) && self.guide_chat.is_some() {
            self.nav_guide_chat(action);
            return;
        }
        if let Some(ModalState::Guide { tab: tab_now, focus: focus_now }) = self.modal {
            let entries = self.guide_entries(tab_now);
            let Some(ModalState::Guide { tab, focus }) = &mut self.modal else { return };
            let items = entries.len();
            match action {
                NavAction::Up if focus_now > 0 => {
                    *focus = if focus_now >= items { items.saturating_sub(1) } else { focus_now - 1 }
                }
                NavAction::Down if focus_now + 1 < items => *focus = focus_now + 1,
                NavAction::Down if focus_now < items => *focus = items,
                NavAction::Left if focus_now > items => *focus = focus_now - 1,
                NavAction::Right if focus_now >= items && focus_now + 1 < items + GUIDE_BUTTONS.len() => {
                    *focus = focus_now + 1
                }
                NavAction::PrevTab => {
                    *tab = tab_now.saturating_sub(1);
                    *focus = 0;
                }
                NavAction::NextTab => {
                    *tab = (tab_now + 1).min(GUIDE_TABS.len() - 1);
                    *focus = 0;
                }
                NavAction::Back => self.close_guide(),
                NavAction::Accept => {
                    let act = match entries.get(focus_now) {
                        Some(e) => e.action.clone(),
                        None => GUIDE_BUTTONS[focus_now - items].1.clone(),
                    };
                    self.modal = None;
                    self.last_guide = (tab_now, focus_now);
                    self.guide_action(act);
                }
                _ => {}
            }
            return;
        }
        if matches!(self.modal, Some(ModalState::Audio { .. })) {
            self.nav_audio(action);
            return;
        }
        if matches!(self.modal, Some(ModalState::Viewer { .. })) {
            self.nav_viewer(action);
            return;
        }
        if matches!(self.modal, Some(ModalState::Power { .. })) {
            self.nav_power(action);
            return;
        }
        if matches!(self.modal, Some(ModalState::Friend { .. })) {
            self.nav_friend(action);
            return;
        }
        if let Some(ModalState::Chats { focus, tab, guide_focus }) = self.modal {
            let peers = self.recent_chats(7);
            let moved = match action {
                NavAction::Up => list_move(focus, peers.len(), -1),
                NavAction::Down => list_move(focus, peers.len(), 1),
                NavAction::Back | NavAction::Left => {
                    self.modal = Some(ModalState::Guide { tab, focus: guide_focus });
                    return;
                }
                NavAction::Accept => {
                    if let Some(&peer) = peers.get(focus) {
                        self.modal = None;
                        self.guide_action(GuideAction::OpenChat(peer));
                    }
                    return;
                }
                _ => focus,
            };
            self.modal = Some(ModalState::Chats { focus: moved, tab, guide_focus });
            return;
        }
        let Some(modal) = &mut self.modal else { return };
        match modal {
            ModalState::Guide { .. }
            | ModalState::Audio { .. }
            | ModalState::Viewer { .. }
            | ModalState::Power { .. }
            | ModalState::Friend { .. }
            | ModalState::Chats { .. } => {}
            ModalState::Menu { entries, focus, .. } => match action {
                NavAction::Up => *focus = list_move(*focus, entries.len(), -1),
                NavAction::Down => *focus = list_move(*focus, entries.len(), 1),
                NavAction::Back | NavAction::Menu => self.modal = None,
                NavAction::Accept => {
                    let act = entries[*focus].action.clone();
                    self.modal = None;
                    self.menu_action(act);
                }
                _ => {}
            },
            ModalState::Picker { kind, focus } => {
                let (count, cols) = picker_shape(*kind, self.avatar_pack.len());
                match action {
                    NavAction::Up | NavAction::Down | NavAction::Left | NavAction::Right => {
                        if let Some(f) = crate::nav::grid_move(*focus, count, cols, action) {
                            *focus = f;
                        }
                    }
                    NavAction::Back => {
                        let kind = *kind;
                        self.modal = None;
                        self.picker_cancelled(kind);
                    }
                    NavAction::Accept => {
                        let (kind, index) = (*kind, *focus);
                        self.modal = None;
                        self.picker_chosen(kind, index);
                    }
                    _ => {}
                }
            }
            ModalState::Keyboard { osk, .. } => {
                let outcome = osk.handle(action);
                self.osk_outcome(outcome);
            }
        }
    }

    fn guide_entries(&self, tab: usize) -> Vec<GuideEntry> {
        let mut out = Vec::new();
        let mut y = 150.0;
        let push = |out: &mut Vec<GuideEntry>, y: &mut f32, e: GuideEntry| {
            out.push(GuideEntry { y: *y, ..e });
            *y += 108.0;
        };
        let row = |glyph: &'static str, label: &str, action: GuideAction| GuideEntry {
            y: 0.0,
            glyph,
            label: label.into(),
            subtitle: String::new(),
            image: None,
            solid: false,
            thumb: false,
            divider_above: false,
            round: false,
            presence: -1,
            action,
            others: Vec::new(),
            more: String::new(),
            profile_row: false,
            score: false,
        };
        if tab == 0 {
            if let Some(game) = self.running_entry() {
                push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: "Resume".into(),
                        image: game.game.media.cover.clone(),
                        thumb: true,
                        ..row("\u{E768}", &game.game.name, GuideAction::Resume)
                    },
                );
                if self.ingame.as_ref().is_some_and(|g| g.capture.is_some()) {
                    push(&mut out, &mut y, row("\u{E722}", "Take screenshot", GuideAction::Screenshot));
                }
                push(&mut out, &mut y, row("\u{E767}", "Audio", GuideAction::Audio));
                push(&mut out, &mut y, row("\u{E711}", "Quit game", GuideAction::QuitGame));
                y += 32.0;
            }
            let top = out.len();
            push(&mut out, &mut y, GuideEntry { divider_above: top > 0, ..row("\u{E80F}", "Home", GuideAction::Home) });
            push(&mut out, &mut y, row("\u{E8F1}", "My games & apps", GuideAction::Library));
            push(&mut out, &mut y, row("\u{E722}", "Captures", GuideAction::Captures));
            push(&mut out, &mut y, row("\u{E719}", "Store", GuideAction::Store));
            push(&mut out, &mut y, row("\u{E7FC}", "Game Pass", GuideAction::GamePass));
            y += 32.0; // divider
            let recent = self.profile_id().map(|p| self.library.recent(p, 3).unwrap_or_default()).unwrap_or_default();
            for (i, e) in recent.into_iter().enumerate() {
                push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: e.game.provider.display_name().into(),
                        image: e.game.media.cover.clone(),
                        thumb: true,
                        divider_above: i == 0,
                        ..row("\u{E7FC}", &e.game.name, GuideAction::Game(e.game.id.clone()))
                    },
                );
            }
            let first_after_divider = out.len() == top + 5;
            push(
                &mut out,
                &mut y,
                GuideEntry {
                    solid: true,
                    thumb: true,
                    divider_above: first_after_divider,
                    ..row("\u{E713}", "Settings", GuideAction::Settings(0))
                },
            );
        } else if tab == TAB_PEOPLE {
            match &self.social_status {
                social::Status::Connected { .. } => {
                    push(&mut out, &mut y, row("\u{E8FA}", "Add a friend", GuideAction::AddFriend));
                    for r in &self.requests {
                        push(
                            &mut out,
                            &mut y,
                            GuideEntry {
                                subtitle: "Wants to be your friend".into(),
                                image: r.avatar.clone(),
                                thumb: true,
                                round: true,
                                ..row("\u{E77B}", &r.name, GuideAction::FriendRequest(r.id))
                            },
                        );
                    }
                    if self.friends.is_empty() {
                        push(&mut out, &mut y, row("\u{E716}", "No friends yet", GuideAction::None));
                    }
                    for (i, f) in self.friends.iter().enumerate() {
                        push(
                            &mut out,
                            &mut y,
                            GuideEntry {
                                subtitle: f.status_line(),
                                image: f.avatar.clone(),
                                thumb: true,
                                round: true,
                                divider_above: i == 0,
                                presence: presence_code(f.presence),
                                ..row("\u{E77B}", &f.name, GuideAction::Friend(f.id))
                            },
                        );
                    }
                }
                social::Status::Connecting => {
                    push(&mut out, &mut y, row("\u{E716}", "Connecting to Discord…", GuideAction::None))
                }
                social::Status::Failed(e) => push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: e.clone(),
                        ..row("\u{E783}", "Couldn't connect to Discord", GuideAction::LinkDiscord)
                    },
                ),
                social::Status::NotLinked => push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: "See friends, chat and start parties".into(),
                        ..row("\u{E716}", "Link Discord", GuideAction::LinkDiscord)
                    },
                ),
            }
        } else if tab == TAB_PARTY {
            if !matches!(self.social_status, social::Status::Connected { .. }) {
                push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: "Parties and chats use Discord".into(),
                        ..row("\u{E716}", "Link Discord", GuideAction::LinkDiscord)
                    },
                );
                return out;
            }
            if let Some((from, name)) = &self.party_invite {
                push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: "Party invite".into(),
                        ..row("\u{E8FA}", &format!("Join {name}'s party"), GuideAction::JoinParty(*from))
                    },
                );
            }
            match &self.party {
                Some(party) => {
                    let (glyph, label) =
                        if party.self_muted { ("\u{F781}", "Unmute myself") } else { ("\u{E720}", "Mute myself") };
                    push(&mut out, &mut y, row(glyph, label, GuideAction::ToggleMute));
                    let label = if party.self_deafened { "Undeafen" } else { "Deafen" };
                    // "deafened" draws the headphones with a slash (no such glyph in the icon font).
                    let glyph = if party.self_deafened { "deafened" } else { "\u{E7F6}" };
                    push(&mut out, &mut y, row(glyph, label, GuideAction::ToggleDeafen));
                    for m in &party.members {
                        let subtitle = if m.muted {
                            "Muted"
                        } else if m.speaking {
                            "Speaking"
                        } else {
                            "In party"
                        };
                        push(
                            &mut out,
                            &mut y,
                            GuideEntry {
                                subtitle: subtitle.into(),
                                image: m.avatar.clone(),
                                thumb: true,
                                round: true,
                                presence: if m.speaking { 0 } else { -1 },
                                ..row("\u{E77B}", &m.name, GuideAction::Friend(m.id))
                            },
                        );
                    }
                    if party.chat != 0 {
                        push(&mut out, &mut y, row("\u{E8BD}", "Party chat", GuideAction::OpenChat(party.chat)));
                    }
                    push(&mut out, &mut y, row("\u{E7E8}", "Leave party", GuideAction::LeaveParty));
                }
                None => push(&mut out, &mut y, row("\u{E710}", "Start a party", GuideAction::StartParty)),
            }
            let recent = self.recent_chats(7);
            let names: Vec<String> = recent
                .iter()
                .map(|peer| self.person(*peer).map_or_else(|| "Discord user".into(), |f| f.name))
                .collect();
            push(
                &mut out,
                &mut y,
                GuideEntry {
                    subtitle: if names.is_empty() { "No recent chats".into() } else { names.join(", ") },
                    divider_above: true,
                    ..row("\u{E8BD}", "Chats", GuideAction::Chats)
                },
            );
        } else if tab == TAB_ACHIEVEMENTS {
            let Some(ach) = &self.ach_guide else {
                push(&mut out, &mut y, row("trophy", "Loading achievements…", GuideAction::None));
                return out;
            };
            push(
                &mut out,
                &mut y,
                GuideEntry {
                    subtitle: "Gamerscore · View your profile".into(),
                    score: true,
                    ..row("", &self.gamerscore.unwrap_or(0).to_string(), GuideAction::MyProfile)
                },
            );
            for (e, s) in &ach.games {
                push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: format!("{}/{} · {}/{} G", s.unlocked, s.total, s.gamerscore, s.max_gamerscore),
                        image: e.game.media.cover.clone(),
                        thumb: true,
                        ..row("\u{E7FC}", &e.game.name, GuideAction::Details(e.game.id.clone()))
                    },
                );
            }
            if ach.games.is_empty() && ach.recent.is_empty() {
                push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: "Set up the Playnite Achievements add-on in Playnite".into(),
                        ..row("trophy", "No achievements yet", GuideAction::None)
                    },
                );
            }
            for (i, (game_id, r)) in ach.recent.iter().enumerate() {
                push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: format!("{} · {} G", r.game, r.achievement.gamerscore),
                        image: r.achievement.icon.clone(),
                        thumb: true,
                        divider_above: i == 0,
                        ..row(
                            "\u{E734}",
                            &r.achievement.name,
                            game_id.clone().map_or(GuideAction::None, GuideAction::Details),
                        )
                    },
                );
            }
        } else if tab == TAB_CAPTURE {
            if self.ingame.as_ref().is_some_and(|g| g.capture.is_some()) {
                push(&mut out, &mut y, row("\u{E722}", "Take screenshot", GuideAction::Screenshot));
            }
            match &self.recording {
                Some(r) => {
                    let secs = r.started.elapsed().as_secs();
                    push(
                        &mut out,
                        &mut y,
                        GuideEntry {
                            subtitle: format!("Recording · {}:{:02}", secs / 60, secs % 60),
                            ..row("\u{E71A}", "Stop recording", GuideAction::StopRecording)
                        },
                    )
                }
                None => push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: "Records the screen until you stop it".into(),
                        ..row("\u{E7C8}", "Start recording", GuideAction::StartRecording)
                    },
                ),
            }
            for (i, c) in self.captures.iter().take(4).enumerate() {
                let video = is_video(c);
                push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: if video { "Recording" } else { "Screenshot" }.into(),
                        image: (!video).then(|| c.clone()),
                        thumb: true,
                        divider_above: i == 0,
                        ..row(
                            if video { "\u{E714}" } else { "\u{E722}" },
                            &c.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                            GuideAction::OpenCapture(i),
                        )
                    },
                );
            }
            push(&mut out, &mut y, row("\u{E8B9}", "See all captures", GuideAction::Captures));
        } else if tab == TAB_NOTIFICATIONS {
            if self.notices.is_empty() {
                push(&mut out, &mut y, row("", "No new notifications", GuideAction::None));
            }
            for n in &self.notices {
                let when = time_ago(n.at);
                push(
                    &mut out,
                    &mut y,
                    GuideEntry {
                        subtitle: if n.body.is_empty() { when } else { format!("{} · {when}", n.body) },
                        image: n.image.clone(),
                        round: n.round && n.image.is_some(),
                        thumb: !n.round && n.image.is_some(),
                        ..row(n.glyph, &n.title, n.action.clone())
                    },
                );
            }
            if !self.notices.is_empty() {
                push(
                    &mut out,
                    &mut y,
                    GuideEntry { divider_above: true, ..row("\u{E894}", "Clear all", GuideAction::ClearNotices) },
                );
            }
        } else {
            let me = self.current.as_ref().map(|p| p.id);
            let name = self.current.as_ref().map_or_else(|| "Profile".to_string(), |p| p.name.clone());
            let others: Vec<Profile> = self.profiles.iter().filter(|p| Some(p.id) != me).cloned().collect();
            let more = if others.len() > 3 { format!("+{}", others.len() - 3) } else { String::new() };
            push(
                &mut out,
                &mut y,
                GuideEntry {
                    others: others.into_iter().take(3).collect(),
                    more,
                    profile_row: true,
                    // Keeps the label on the upper line; the other profiles' pictures sit below it.
                    subtitle: " ".into(),
                    ..row("\u{E748}", &name, GuideAction::MyProfile)
                },
            );
            push(&mut out, &mut y, row("", "Sign out", GuideAction::SwitchProfile));
        }
        out
    }

    fn guide_action(&mut self, action: GuideAction) {
        tracing::debug!("guide action {action:?}");
        if matches!(
            action,
            GuideAction::Home
                | GuideAction::Library
                | GuideAction::Game(_)
                | GuideAction::Settings(_)
                | GuideAction::SwitchProfile
                | GuideAction::MyProfile
                | GuideAction::Search
                | GuideAction::AddGame
                | GuideAction::LinkDiscord
                | GuideAction::Store
                | GuideAction::GamePass
                | GuideAction::Details(_)
        ) {
            self.leave_ingame();
        }
        match action {
            GuideAction::Resume => {
                if self.ingame.is_some() {
                    self.close_guide();
                } else {
                    self.resume_game();
                }
            }
            GuideAction::Screenshot => {
                self.modal = Some(ModalState::Guide { tab: 0, focus: 1 });
                self.save_screenshot();
            }
            GuideAction::Audio => self.open_audio(),
            GuideAction::Captures => self.open_captures(0),
            GuideAction::Details(id) => self.open_game_page(&id),
            GuideAction::AddFriend => {
                self.open_keyboard("Add a friend by their Discord username", "", 40, OskPurpose::AddFriend)
            }
            GuideAction::FriendRequest(id) => {
                let name = self.requests.iter().find(|r| r.id == id).map_or("Someone".into(), |r| r.name.clone());
                self.open_menu(
                    &name,
                    "Wants to be your friend on Discord",
                    vec![
                        entry("\u{E8FB}", "Accept", MenuAction::AnswerRequest(id, true)),
                        entry("\u{E711}", "Decline", MenuAction::AnswerRequest(id, false)),
                    ],
                );
            }
            GuideAction::Chats => {
                let (tab, guide_focus) = self.last_guide;
                if self.recent_chats(7).is_empty() {
                    self.toast("No recent chats. Message a friend from People.");
                    self.modal = Some(ModalState::Guide { tab, focus: guide_focus });
                } else {
                    self.modal = Some(ModalState::Chats { focus: 0, tab, guide_focus });
                }
            }
            GuideAction::OpenCapture(i) => {
                self.refresh_captures();
                if i < self.captures.len() {
                    self.modal = Some(ModalState::Viewer { index: i });
                }
            }
            GuideAction::StartRecording => self.start_recording(),
            GuideAction::StopRecording => {
                self.stop_recording();
                self.modal = Some(ModalState::Guide { tab: TAB_CAPTURE, focus: 0 });
            }
            GuideAction::Store => self.open_browse("Store", "\u{E719}", "https://store.steampowered.com/"),
            GuideAction::GamePass => {
                self.open_browse("Game Pass", "\u{E7FC}", "https://www.xbox.com/xbox-game-pass/games")
            }
            GuideAction::QuitGame => {
                let name = self.running_entry().map(|e| e.game.name).unwrap_or_else(|| "the game".into());
                self.open_menu(
                    &format!("Quit {name}?"),
                    "Any unsaved progress will be lost.",
                    vec![
                        entry("\u{E711}", "Quit", MenuAction::QuitGame),
                        entry("\u{E72B}", "Cancel", MenuAction::Close),
                    ],
                );
            }
            GuideAction::Home => {
                self.home_section = 0;
                self.go(View::Home);
            }
            GuideAction::Library => self.open_library(),
            GuideAction::Game(id) => {
                if let Some(e) = self.entry_by_id(&id) {
                    self.play(&e);
                }
            }
            GuideAction::Settings(c) => self.open_settings(c),
            GuideAction::SwitchProfile => self.switch_profile(),
            GuideAction::MyProfile => self.open_my_profile(),
            GuideAction::Keyboard => {
                // Back to the game (or the desktop) first, so the keys go to it.
                if self.ingame.take().is_some() {
                    self.resume_game();
                }
                self.open_vkeyboard();
            }
            GuideAction::ClearNotices => {
                self.notices.clear();
                self.modal = Some(ModalState::Guide { tab: TAB_NOTIFICATIONS, focus: 0 });
            }
            GuideAction::Search => self.open_search(),
            GuideAction::AddGame => {
                self.open_files("Choose the game's .exe", Filter::Executables, FilePurpose::AddGame)
            }
            GuideAction::Power => self.open_power_menu(),
            GuideAction::None => self.modal = Some(ModalState::Guide { tab: TAB_PEOPLE, focus: 0 }),
            GuideAction::LinkDiscord => {
                self.settings_category = 1;
                self.start_sign_in(Service::Discord);
            }
            GuideAction::Friend(id) => {
                let (tab, guide_focus) = self.last_guide;
                self.modal = Some(ModalState::Friend { id, focus: 0, tab, guide_focus });
            }
            GuideAction::OpenChat(peer) => {
                if let Some(s) = &mut self.social {
                    s.load_history(peer);
                }
                self.modal = Some(ModalState::Guide { tab: TAB_PARTY, focus: 0 });
                self.guide_chat = Some(GuideChat { peer, compose: None, scroll: 0 });
            }
            GuideAction::StartParty => {
                let online: Vec<MenuEntry> = self
                    .friends
                    .iter()
                    .filter(|f| f.presence != Presence::Offline)
                    .map(|f| {
                        entry(
                            "\u{E8FA}",
                            &format!("Invite {}", f.name),
                            MenuAction::Guide(GuideAction::JoinParty(f.id)),
                        )
                    })
                    .collect();
                if online.is_empty() {
                    self.toast("None of your friends are online");
                } else {
                    self.open_menu("Start a party", "Choose a friend to invite.", online);
                }
            }
            GuideAction::JoinParty(peer) => {
                let invited_me = self.party_invite.as_ref().is_some_and(|(from, _)| *from == peer);
                if let Some(s) = &mut self.social {
                    if invited_me {
                        s.accept_party_invite(peer);
                    } else {
                        s.invite_to_party(peer);
                    }
                }
                if invited_me {
                    self.party_invite = None;
                }
                self.modal = Some(ModalState::Guide { tab: TAB_PARTY, focus: 0 });
            }
            GuideAction::ToggleMute => {
                let muted = self.party.as_ref().is_some_and(|p| p.self_muted);
                if let Some(s) = &mut self.social {
                    s.set_muted(!muted);
                }
                self.modal = Some(ModalState::Guide { tab: TAB_PARTY, focus: 0 });
            }
            GuideAction::ToggleDeafen => {
                let deaf = self.party.as_ref().is_some_and(|p| p.self_deafened);
                if let Some(s) = &mut self.social {
                    s.set_deafened(!deaf);
                }
                self.modal = Some(ModalState::Guide { tab: TAB_PARTY, focus: 1 });
            }
            GuideAction::LeaveParty => {
                if let Some(s) = &mut self.social {
                    s.leave_party();
                }
                self.modal = Some(ModalState::Guide { tab: TAB_PARTY, focus: 0 });
            }
        }
    }

    // ================================================================ social

    /// People you've messaged, most recent conversation first.
    fn recent_chats(&self, n: usize) -> Vec<u64> {
        let mut convos: Vec<(u64, i64)> =
            self.chats.iter().filter_map(|(peer, m)| m.last().map(|last| (*peer, last.at))).collect();
        convos.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        convos.into_iter().take(n).map(|(p, _)| p).collect()
    }

    fn chats_file(&self) -> Option<PathBuf> {
        self.profile_id().map(|id| self.store.profile_dir(id).join("chats.bin"))
    }

    /// Message history kept on this PC (encrypted for the Windows user), so chats older than Discord's
    /// 72-hour window stay visible.
    fn load_chats(&mut self) {
        self.chats = self
            .chats_file()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|enc| cs_platform::secret::unprotect(&enc).ok())
            .and_then(|json| serde_json::from_slice(&json).ok())
            .unwrap_or_default();
    }

    fn save_chats_soon(&mut self) {
        if !self.chat_save_timer.running() {
            self.chat_save_timer.start(slint::TimerMode::SingleShot, Duration::from_secs(2), || {
                crate::with_app(|app| app.save_chats());
            });
        }
    }

    fn save_chats(&mut self) {
        let Some(path) = self.chats_file() else { return };
        for msgs in self.chats.values_mut() {
            let extra = msgs.len().saturating_sub(300);
            msgs.drain(..extra);
        }
        let result = serde_json::to_vec(&self.chats)
            .map_err(std::io::Error::other)
            .and_then(|json| cs_platform::secret::protect(&json))
            .and_then(|enc| std::fs::write(&path, enc));
        if let Err(e) = result {
            tracing::warn!("saving chat history: {e}");
        }
    }

    fn start_social(&mut self) {
        self.stop_social();
        self.load_chats();
        if std::env::var("WIN2CONSOLE_DEMO_SOCIAL").is_ok_and(|v| v == "1") {
            self.social = Some(Box::new(social::DemoBackend::new(|ev| post(move |app| app.social_event(ev)))));
        } else if let Some(token) =
            self.discord_account().and_then(|a| a.tokens["refresh_token"].as_str().map(str::to_string))
        {
            if let Some(mut backend) = self.new_discord_backend() {
                backend.resume(&token);
                self.social = Some(Box::new(backend));
            }
        }
        self.social_timer.start(slint::TimerMode::Repeated, Duration::from_millis(50), || {
            crate::with_app(|app| {
                if let Some(s) = &mut app.social {
                    s.tick();
                }
            });
        });
    }

    fn discord_app_id(&self) -> u64 {
        self.config.discord_app_id.as_deref().unwrap_or(DEFAULT_DISCORD_APP_ID).parse().unwrap_or(0)
    }

    fn discord_account(&self) -> Option<Account> {
        let id = self.profile_id()?;
        AccountStore::for_profile(&self.store.profile_dir(id)).load().remove("discord")
    }

    /// Saves (or forgets, on `None`) the Discord refresh token for the signed-in profile.
    fn save_discord_token(&mut self, token: Option<String>) {
        let Some(id) = self.profile_id() else { return };
        let store = AccountStore::for_profile(&self.store.profile_dir(id));
        let mut accounts = store.load();
        match token {
            Some(t) => {
                let entry = accounts.entry("discord".into()).or_insert_with(|| Account {
                    id: String::new(),
                    name: String::new(),
                    tokens: serde_json::Value::Null,
                });
                entry.tokens = serde_json::json!({ "refresh_token": t });
            }
            None => {
                accounts.remove("discord");
            }
        }
        if let Err(e) = store.save(&accounts) {
            tracing::warn!("saving Discord sign-in: {e}");
        }
    }

    #[cfg(discord_sdk)]
    fn new_discord_backend(&self) -> Option<crate::discord::DiscordBackend> {
        Some(crate::discord::DiscordBackend::new(
            self.discord_app_id(),
            |ev| post(move |app| app.social_event(ev)),
            |token| post(move |app| app.save_discord_token(token)),
        ))
    }

    #[cfg(not(discord_sdk))]
    fn new_discord_backend(&self) -> Option<social::DemoBackend> {
        tracing::warn!("built without the Discord Social SDK");
        None
    }

    /// Records a notification for the Guide and shows it (over the game too), with the notification sound.
    fn notify(&mut self, notice: Notice) {
        crate::sounds::play(crate::sounds::Sound::Notify);
        let picture = notice.image.as_ref().and_then(|p| self.images.get(p, Size::Tile));
        let text = if notice.body.is_empty() { notice.title.clone() } else { format!("{}: {}", notice.title, notice.body) };
        if self.running.is_some() && !cs_platform::window::own_window_is_foreground() {
            self.notify_over_game_with(&text, picture);
        }
        self.toast_in_app(&text);
        self.notices.insert(0, notice);
        self.notices.truncate(NOTICES_MAX);
    }

    /// A notification about a person (party joins, invites), with their picture.
    fn notify_person(&mut self, id: u64, glyph: &'static str, title: String) {
        let image = self.person(id).and_then(|f| f.avatar);
        let notice = Notice { glyph, title, body: String::new(), image, round: true, at: cs_core::now_unix(), action: GuideAction::None };
        self.notify(notice);
    }

    fn open_vkeyboard(&mut self) {
        use slint::ComponentHandle;
        let window = match crate::ui::KeyboardWindow::new() {
            Ok(w) => w,
            Err(e) => return tracing::warn!("keyboard window: {e}"),
        };
        cs_input::set_mouse_mode(false);
        let (sw, sh) = cs_platform::window::screen_size();
        // Design units → physical pixels: about 85% of the dashboard's size.
        let k = sh as f32 / 1080.0 * 0.85;
        let (ww, wh) = ((1046.0 * k) as u32, (380.0 * k) as u32);
        window.set_ui_scale(k / self.ui().window().scale_factor());
        window.window().set_size(slint::PhysicalSize::new(ww, wh));
        window.window().set_position(slint::PhysicalPosition::new((sw - ww as i32) / 2, sh - wh as i32 - sh / 20));
        // Showing a window activates it; hand the focus straight back to whatever we're typing into.
        let target = cs_platform::window::foreground_window();
        if let Err(e) = window.show() {
            return tracing::warn!("keyboard window: {e}");
        }
        if let Some(hwnd) = overlay_hwnd(window.window()) {
            cs_platform::window::make_overlay(hwnd);
        }
        cs_platform::window::set_foreground_window(target);
        let mut osk = Osk::new("Keyboard", "", 1);
        osk.max_len = usize::MAX;
        self.vkeyboard = Some((osk, window));
        self.render_vkeyboard();
    }

    fn close_vkeyboard(&mut self) {
        use slint::ComponentHandle;
        if let Some((_, w)) = self.vkeyboard.take() {
            let _ = w.hide();
        }
        if self.desktop {
            cs_input::set_mouse_mode(true);
        }
    }

    /// The virtual keyboard: keys go to the window in front as real key presses.
    fn nav_vkeyboard(&mut self, action: NavAction) {
        use crate::osk::KeyAction;
        use cs_platform::input::{self, VK_BACK, VK_LEFT, VK_RETURN, VK_RIGHT};
        use crate::sounds::{Sound, play};
        let Some((osk, _)) = &mut self.vkeyboard else { return };
        match action {
            NavAction::Back => {
                play(Sound::Back);
                return self.close_vkeyboard();
            }
            NavAction::Guide => {
                self.close_vkeyboard();
                play(Sound::Guide);
                return self.toggle_guide();
            }
            NavAction::X => input::press_key(VK_BACK),
            NavAction::Y => input::type_text(" "),
            NavAction::PrevTab => input::press_key(VK_LEFT),
            NavAction::NextTab => input::press_key(VK_RIGHT),
            NavAction::Menu => input::press_key(VK_RETURN),
            NavAction::Accept => {
                play(Sound::Select);
                match osk.keys().get(osk.focus).map(|k| k.action) {
                    Some(KeyAction::Char(c)) => {
                        input::type_text(&c.to_string());
                        // Keeps the keyboard's one-shot Shift behaviour; nothing is kept here.
                        osk.type_char(c);
                        osk.clear();
                    }
                    Some(KeyAction::Space) => input::type_text(" "),
                    Some(KeyAction::Backspace) => input::press_key(VK_BACK),
                    Some(KeyAction::CursorLeft) => input::press_key(VK_LEFT),
                    Some(KeyAction::CursorRight) => input::press_key(VK_RIGHT),
                    Some(KeyAction::Done) => input::press_key(VK_RETURN),
                    Some(KeyAction::Shift | KeyAction::Symbols) => {
                        let focus = osk.focus;
                        osk.press(focus);
                    }
                    None => {}
                }
            }
            _ => {
                if action.is_direction() {
                    play(Sound::Move);
                }
                osk.handle(action);
            }
        }
        self.render_vkeyboard();
    }

    fn render_vkeyboard(&self) {
        let Some((osk, w)) = &self.vkeyboard else { return };
        w.set_keys(Rc::new(VecModel::from(key_views(osk))).into());
        w.set_selected(osk.focus as i32);
    }

    /// Starts watching the Playnite Achievements add-on's database for new unlocks.
    pub fn start_achievement_watch(&mut self) {
        self.poll_achievements();
        self.ach_timer.start(slint::TimerMode::Repeated, Duration::from_secs(15), || {
            crate::with_app(|app| app.poll_achievements());
        });
    }

    fn poll_achievements(&mut self) {
        std::thread::spawn(|| {
            let recent = cs_library::achievements::Achievements::open().map(|a| a.recent_unlocks(40));
            post(move |app| app.achievements_polled(recent));
        });
    }

    fn achievements_polled(&mut self, recent: Option<Vec<cs_library::achievements::RecentUnlock>>) {
        let Some(recent) = recent else { return };
        let key = |r: &cs_library::achievements::RecentUnlock| (r.game.clone(), r.achievement.name.clone());
        let Some(seen) = &mut self.ach_seen else {
            // First look: everything already unlocked counts as seen.
            self.ach_seen = Some(recent.iter().map(key).collect());
            return;
        };
        let new: Vec<_> = recent.into_iter().rev().filter(|r| seen.insert(key(r))).collect();
        if new.is_empty() {
            return;
        }
        for r in new {
            let a = &r.achievement;
            self.notices.insert(
                0,
                Notice {
                    glyph: "trophy",
                    title: a.name.clone(),
                    body: format!("{} G · {}", a.gamerscore, r.game),
                    image: a.icon.clone(),
                    round: false,
                    at: cs_core::now_unix(),
                    action: GuideAction::None,
                },
            );
            self.ach_queue.push_back(r);
        }
        self.notices.truncate(NOTICES_MAX);
        self.refresh_gamerscore();
        if self.ach_popup.as_ref().is_none_or(|w| !w.window().is_visible()) {
            self.next_achievement_popup();
        }
    }

    /// Shows the next queued achievement, Xbox style: icon, gamerscore, name and description.
    fn next_achievement_popup(&mut self) {
        use slint::ComponentHandle;
        let Some(r) = self.ach_queue.pop_front() else {
            if let Some(w) = &self.ach_popup {
                let _ = w.hide();
            }
            return;
        };
        crate::sounds::play(crate::sounds::Sound::Achievement);
        if self.ach_popup.is_none() {
            match crate::ui::AchievementWindow::new() {
                Ok(w) => self.ach_popup = Some(w),
                Err(e) => return tracing::warn!("achievement window: {e}"),
            }
        }
        let icon = r.achievement.icon.as_ref().and_then(|p| self.images.get(p, Size::Tile));
        let Some(w) = &self.ach_popup else { return };
        w.set_has_badge(icon.is_some());
        w.set_badge(icon.unwrap_or_default());
        w.set_score(r.achievement.gamerscore.to_string().into());
        w.set_name(r.achievement.name.clone().into());
        w.set_description(r.achievement.description.clone().into());
        let (sw, sh) = cs_platform::window::screen_size();
        let (ww, wh) = ((sh as f32 * 0.66) as u32, (sh as f32 * 0.1) as u32);
        w.window().set_size(slint::PhysicalSize::new(ww, wh));
        w.window().set_position(slint::PhysicalPosition::new((sw - ww as i32) / 2, sh - wh as i32 - sh / 12));
        if let Err(e) = w.show() {
            return tracing::warn!("achievement window: {e}");
        }
        if let Some(hwnd) = overlay_hwnd(w.window()) {
            cs_platform::window::make_overlay(hwnd);
            cs_platform::window::round_window(hwnd, ww as i32, wh as i32, wh as i32 / 2);
        }
        self.ach_popup_timer.start(slint::TimerMode::SingleShot, Duration::from_secs(6), || {
            crate::with_app(|app| app.next_achievement_popup());
        });
    }

    /// A friend, or someone we have messages from, by Discord id.
    fn person(&self, id: u64) -> Option<Friend> {
        let friend = self.friends.iter().find(|f| f.id == id).cloned();
        match (friend, self.users.get(&id)) {
            (Some(f), Some(u)) if f.name == "Discord user" => Some(Friend { name: u.name.clone(), ..f }),
            (Some(f), _) => Some(f),
            (None, u) => u.cloned(),
        }
    }

    fn stop_social(&mut self) {
        self.social = None;
        self.users.clear();
        self.social_timer.stop();
        self.social_status = social::Status::NotLinked;
        self.friends.clear();
        self.chats.clear();
        self.party = None;
        self.party_invite = None;
        self.guide_chat = None;
    }

    fn social_event(&mut self, ev: social::Event) {
        match ev {
            social::Event::Status(status) => {
                if let social::Status::Connected { user } = &status {
                    let user = user.clone();
                    let already = self.current.as_ref().is_some_and(|p| {
                        p.accounts.iter().any(|a| a.service == Service::Discord && a.display_name == user)
                    });
                    if !already {
                        self.update_profile(|p| {
                            p.accounts.retain(|a| a.service != Service::Discord);
                            p.accounts.push(cs_core::LinkedAccount {
                                service: Service::Discord,
                                account_id: String::new(),
                                display_name: user,
                            });
                        });
                    }
                }
                self.social_status = status;
            }
            social::Event::Friends(mut friends) => {
                // Discord sometimes lists a friend before it has their details.
                for f in friends.iter_mut().filter(|f| f.name.is_empty()) {
                    f.name = self.users.get(&f.id).map_or_else(|| "Discord user".into(), |u| u.name.clone());
                    if f.avatar.is_none() {
                        f.avatar = self.users.get(&f.id).and_then(|u| u.avatar.clone());
                    }
                }
                friends.sort_by(|a, b| {
                    a.presence.cmp(&b.presence).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                self.friends = friends;
            }
            social::Event::Message { peer, message } => {
                let viewing = self.guide_chat.as_ref().is_some_and(|c| c.peer == peer)
                    && matches!(self.modal, Some(ModalState::Guide { .. }));
                if !message.from_me && !viewing && cs_core::now_unix() - message.at < 60 {
                    let who = self.person(peer);
                    let party = self.party.as_ref().is_some_and(|p| p.chat == peer);
                    let title = if party { "Party chat".into() } else { who.as_ref().map_or("Message".into(), |f| f.name.clone()) };
                    let body = crate::gifs::find_link(&message.text)
                        .map_or_else(|| message.text.clone(), |(_, rest)| if rest.is_empty() { "Sent a GIF".into() } else { rest });
                    self.notify(Notice {
                        glyph: "\u{E8BD}",
                        title,
                        body,
                        image: who.and_then(|f| f.avatar),
                        round: true,
                        at: message.at,
                        action: GuideAction::OpenChat(peer),
                    });
                }
                let chat = self.chats.entry(peer).or_default();
                if !chat.iter().any(|m| m.id == message.id) {
                    chat.push(message);
                    chat.sort_by_key(|m| m.at);
                    self.save_chats_soon();
                }
            }
            social::Event::Party(party) => {
                // Tell the user when someone joins or leaves the party.
                let before: Vec<(u64, String)> =
                    self.party.iter().flat_map(|p| &p.members).map(|m| (m.id, m.name.clone())).collect();
                let after: Vec<(u64, String)> =
                    party.iter().flat_map(|p| &p.members).map(|m| (m.id, m.name.clone())).collect();
                for (id, name) in &after {
                    if !before.iter().any(|(b, _)| b == id) {
                        self.notify_person(*id, "\u{E716}", format!("{name} joined the party"));
                    }
                }
                for (id, name) in &before {
                    if party.is_some() && !after.iter().any(|(a, _)| a == id) {
                        self.notify_person(*id, "\u{E716}", format!("{name} left the party"));
                    }
                }
                self.party = party;
            }
            social::Event::PartyInvite { from, name } => {
                self.notify_person(from, "\u{E716}", format!("{name} invited you to a party. Open the Guide to join."));
                self.party_invite = Some((from, name));
            }
            social::Event::Requests(requests) => {
                let new: Vec<Friend> =
                    requests.iter().filter(|r| !self.requests.iter().any(|o| o.id == r.id)).cloned().collect();
                for r in new {
                    self.notify(Notice {
                        glyph: "\u{E8FA}",
                        title: format!("{} sent you a friend request", r.name),
                        body: String::new(),
                        image: r.avatar.clone(),
                        round: true,
                        at: cs_core::now_unix(),
                        action: GuideAction::FriendRequest(r.id),
                    });
                }
                self.requests = requests;
            }
            social::Event::Notice(text) => self.toast(text),
            social::Event::User(user) => {
                if !user.name.is_empty() {
                    for f in self.friends.iter_mut().filter(|f| f.id == user.id && f.name == "Discord user") {
                        f.name = user.name.clone();
                        f.avatar = f.avatar.take().or_else(|| user.avatar.clone());
                    }
                    self.users.insert(user.id, user);
                }
            }
            social::Event::Audio(info) => {
                self.discord_audio = Some(info);
                if let Some(ModalState::Audio { focus, .. }) = self.modal {
                    self.open_audio();
                    if let Some(ModalState::Audio { focus: f, sources, .. }) = &mut self.modal {
                        *f = focus.min(sources.len().saturating_sub(1));
                    }
                }
            }
        }
        self.render();
    }

    fn nav_guide_chat(&mut self, action: NavAction) {
        let Some(chat) = &mut self.guide_chat else { return };
        if let Some(osk) = &mut chat.compose {
            let outcome = osk.handle(action);
            self.osk_outcome(outcome);
            return;
        }
        let count = self.chats.get(&chat.peer).map_or(0, Vec::len);
        match action {
            NavAction::Accept => chat.compose = Some(Osk::new("Message", "", 500)),
            NavAction::Back => self.guide_chat = None,
            NavAction::Up => chat.scroll = (chat.scroll + 1).min(count.saturating_sub(1)),
            NavAction::Down => chat.scroll = chat.scroll.saturating_sub(1),
            NavAction::X => chat.scroll = 0,
            NavAction::PrevTab | NavAction::NextTab => {
                self.guide_chat = None;
                self.nav_modal(action);
            }
            _ => {}
        }
    }

    fn open_menu(&mut self, title: &str, message: &str, entries: Vec<MenuEntry>) {
        self.modal = Some(ModalState::Menu { title: title.into(), message: message.into(), entries, focus: 0 });
    }

    fn open_power_menu(&mut self) {
        self.modal = Some(ModalState::Power { focus: 0 });
    }

    /// Actions on a friend's card.
    fn friend_actions(&self, id: u64) -> Vec<(&'static str, &'static str, GuideAction)> {
        let invited_me = self.party_invite.as_ref().is_some_and(|(from, _)| *from == id);
        vec![
            ("\u{E8BD}", "Send message", GuideAction::OpenChat(id)),
            if invited_me {
                ("\u{E8FA}", "Join their party", GuideAction::JoinParty(id))
            } else {
                ("\u{E8FA}", "Invite to party", GuideAction::JoinParty(id))
            },
        ]
    }

    fn nav_friend(&mut self, action: NavAction) {
        let Some(ModalState::Friend { id, focus, tab, guide_focus }) = self.modal else { return };
        let actions = self.friend_actions(id);
        let moved = match action {
            NavAction::Up => list_move(focus, actions.len(), -1),
            NavAction::Down => list_move(focus, actions.len(), 1),
            NavAction::Back | NavAction::Left => {
                self.modal = Some(ModalState::Guide { tab, focus: guide_focus });
                return;
            }
            NavAction::Accept => {
                self.modal = None;
                self.guide_action(actions[focus].2.clone());
                return;
            }
            _ => focus,
        };
        self.modal = Some(ModalState::Friend { id, focus: moved, tab, guide_focus });
    }

    fn nav_power(&mut self, action: NavAction) {
        let Some(ModalState::Power { focus }) = &mut self.modal else { return };
        let n = POWER_ITEMS.len();
        let f = *focus;
        match action {
            // Options are in rows of 3; CLOSE (index n) sits below them.
            NavAction::Left if f < n && f % 3 > 0 => *focus -= 1,
            NavAction::Right if f < n && f % 3 < 2 && f + 1 < n => *focus += 1,
            NavAction::Down if f + 3 < n => *focus += 3,
            NavAction::Down if f < n => *focus = n,
            NavAction::Up if f == n => *focus = (n - 1) / 3 * 3,
            NavAction::Up if f >= 3 => *focus -= 3,
            NavAction::Back => self.modal = None,
            NavAction::Accept => {
                self.modal = None;
                if let Some((_, _, act)) = POWER_ITEMS.get(f) {
                    self.menu_action(act.clone());
                }
            }
            _ => {}
        }
    }

    fn open_game_menu(&mut self, e: &GameEntry) {
        let id = e.game.id.clone();
        let mut entries = vec![
            entry(
                if e.installed { "\u{E768}" } else { "\u{E896}" },
                if e.installed { "Play" } else { "Install" },
                MenuAction::Play(id.clone()),
            ),
            entry("\u{E946}", "Game details & achievements", MenuAction::Details(id.clone())),
            entry(
                "\u{E718}",
                if e.stats.pinned { "Unpin from Home" } else { "Pin to Home" },
                MenuAction::TogglePin(id.clone()),
            ),
            entry("\u{EB9F}", "Change cover art", MenuAction::SetArt(id.clone(), ArtSlot::Cover)),
            entry("\u{EB9F}", "Change background art", MenuAction::SetArt(id.clone(), ArtSlot::Background)),
            entry("\u{ED1A}", if e.stats.hidden { "Unhide" } else { "Hide" }, MenuAction::ToggleHidden(id.clone())),
        ];
        if e.game.provider == Provider::Custom {
            entries.push(entry("\u{E74D}", "Remove from library", MenuAction::Remove(id)));
        }
        let played = format_playtime(e.stats.playtime_secs);
        self.open_menu(&e.game.name, &format!("{} · {played}", e.game.provider.display_name()), entries);
    }

    fn menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::Close => {}
            MenuAction::ToggleShowcase(id) => self.toggle_showcase(id),
            MenuAction::QuitGame => self.quit_game(),
            MenuAction::Details(id) => self.open_game_page(&id),
            MenuAction::Startup(choice) => {
                // The custom video lives in the data folder as startup.mp4 / startup.webm.
                let remove_video = || {
                    for f in ["startup.mp4", "startup.webm"] {
                        let _ = std::fs::remove_file(self.paths.root.join(f));
                    }
                };
                match choice {
                    StartupChoice::BuiltIn => {
                        remove_video();
                        self.config.startup_off = false;
                    }
                    StartupChoice::Off => self.config.startup_off = true,
                    StartupChoice::Video => {
                        self.open_files("Choose a startup video", Filter::Videos, FilePurpose::StartupVideo);
                        return;
                    }
                }
                self.save_config();
            }
            MenuAction::AnswerRequest(id, accept) => {
                if let Some(s) = &mut self.social {
                    s.answer_friend_request(id, accept);
                }
                self.requests.retain(|r| r.id != id);
                self.toast(if accept { "Friend added" } else { "Request declined" });
            }
            MenuAction::CheckUpdates => self.check_updates(!self.config.manual_updates, true),
            MenuAction::ToggleAutoUpdate => {
                self.config.manual_updates = !self.config.manual_updates;
                self.save_config();
                self.toast(if self.config.manual_updates { "Automatic updates off" } else { "Automatic updates on" });
            }
            MenuAction::InstallUpdate => {
                let Some(info) = self.update_info.clone() else { return };
                self.update_state = updater::State::Downloading(info.TargetFullRelease.Version.clone());
                self.toast("Installing the update. Win2Console will restart…");
                std::thread::spawn(move || {
                    if let Err(e) = updater::install_now(&info) {
                        post(move |app| app.toast(format!("Couldn't install the update: {e}")));
                    }
                });
            }
            MenuAction::UiScale(p) => {
                self.config.ui_scale = (p != 100).then_some(p);
                self.save_config();
                self.apply_branding();
            }
            MenuAction::ChooseAppIcon => self.open_files("Choose the app icon", Filter::Images, FilePurpose::AppIcon),
            MenuAction::ResetAppIcon => {
                let _ = std::fs::remove_file(self.paths.root.join(APP_ICON));
                self.apply_branding();
            }
            MenuAction::SetSteamAccount(account) => self.update_profile(|p| {
                p.accounts.retain(|a| a.service != Service::Steam);
                if let Some((account_id, display_name)) = account {
                    p.accounts.push(cs_core::LinkedAccount { service: Service::Steam, account_id, display_name });
                }
            }),
            MenuAction::Play(id) => {
                if let Some(e) = self.entry_by_id(&id) {
                    self.play(&e);
                }
            }
            MenuAction::TogglePin(id) => self.toggle_pin(&id),
            MenuAction::ToggleHidden(id) => self.toggle_hidden(&id),
            MenuAction::SetArt(id, slot) => {
                let title = match slot {
                    ArtSlot::Cover => "Choose cover art",
                    ArtSlot::Background => "Choose background art",
                };
                self.open_files(title, Filter::Images, FilePurpose::Art { game_id: id, slot });
            }
            MenuAction::Remove(id) => {
                let name = self.library.game(&id).ok().flatten().map(|g| g.name).unwrap_or_default();
                self.open_menu(
                    &format!("Remove {name}?"),
                    "The game's files are not deleted.",
                    vec![
                        entry("\u{E74D}", "Remove", MenuAction::ConfirmRemove(id)),
                        entry("\u{E711}", "Cancel", MenuAction::Close),
                    ],
                );
            }
            MenuAction::ConfirmRemove(id) => {
                if let Err(e) = self.library.remove_game(&id) {
                    log_db_err::<()>(e);
                }
                let _ = std::fs::remove_dir_all(self.paths.game_media_dir(&id));
                self.reload();
                self.toast("Removed from library");
            }
            MenuAction::ConfirmDeleteProfile => self.delete_current_profile(),
            MenuAction::Guide(action) => self.guide_action(action),
            MenuAction::SortBy(sort) => {
                self.lib_sort = sort;
                self.lib_focus = 0;
                self.reload_library();
            }
            MenuAction::Sleep => self.power(cs_platform::power::sleep()),
            MenuAction::Restart => self.power(cs_platform::power::restart()),
            MenuAction::ShutDown => self.power(cs_platform::power::shut_down()),
            MenuAction::ExitToDesktop => self.exit_to_desktop(),
        }
    }

    /// Goes to the desktop but keeps running: the controller moves the pointer and the Guide button brings the
    /// Guide back.
    fn exit_to_desktop(&mut self) {
        self.modal = None;
        self.desktop = true;
        cs_input::set_mouse_mode(true);
        cs_platform::window::minimize_own_window();
        self.notify_over_game_with(
            "Left stick moves the pointer, A clicks, X right-clicks, right stick scrolls. Xbox button: Guide.",
            None,
        );
    }

    /// Back from the desktop to Win2Console's own screens.
    fn leave_desktop(&mut self) {
        if self.desktop {
            self.desktop = false;
            cs_input::set_mouse_mode(false);
        }
    }

    fn quit(&mut self) {
        self.stop_tracking();
        let _ = slint::quit_event_loop();
    }

    fn power(&mut self, result: std::io::Result<()>) {
        if let Err(e) = result {
            self.toast(format!("Power action failed: {e}"));
        }
    }

    fn open_picker(&mut self, kind: PickerKind) {
        if matches!(kind, PickerKind::Avatar { .. }) {
            self.load_avatar_pack();
        }
        let focus = match (kind, &self.current) {
            (PickerKind::Accent { .. }, Some(p)) => ACCENTS.iter().position(|(hex, _)| *hex == p.accent).unwrap_or(0),
            (PickerKind::Avatar { .. }, Some(p)) => match p.avatar {
                Avatar::Builtin { index } => (index as usize).min(BUILTIN_AVATARS - 1),
                Avatar::Custom => BUILTIN_AVATARS,
            },
            (PickerKind::Background, Some(p)) => match p.background {
                Background::Default => 0,
                Background::Solid => 1,
                Background::Custom => 2,
            },
            _ => 0,
        };
        self.modal = Some(ModalState::Picker { kind, focus });
    }

    fn picker_chosen(&mut self, kind: PickerKind, index: usize) {
        match kind {
            PickerKind::Captures { .. } => self.modal = Some(ModalState::Viewer { index }),
            PickerKind::Accent { onboarding } => {
                if index == ACCENTS.len() {
                    let current = self.current.as_ref().map(|p| p.accent.clone()).unwrap_or_default();
                    self.open_keyboard("My color (#RRGGBB)", &current, 7, OskPurpose::CustomAccent { onboarding });
                    return;
                }
                self.update_profile(|p| p.accent = ACCENTS[index].0.to_string());
                if onboarding {
                    self.open_picker(PickerKind::Avatar { onboarding: true });
                }
            }
            PickerKind::Avatar { onboarding } => {
                if index == BUILTIN_AVATARS {
                    self.open_files("Choose a profile picture", Filter::Images, FilePurpose::Avatar);
                    return;
                }
                if let Some(path) = self.avatar_pack.get(index.wrapping_sub(BUILTIN_AVATARS + 1)).cloned() {
                    self.file_picked(path, FilePurpose::Avatar);
                    if onboarding {
                        self.toast("You're all set");
                    }
                    return;
                }
                self.update_profile(|p| p.avatar = Avatar::Builtin { index: index as u32 });
                if onboarding {
                    self.toast("You're all set");
                }
            }
            PickerKind::Background => match index {
                0 => self.update_profile(|p| p.background = Background::Default),
                1 => self.update_profile(|p| p.background = Background::Solid),
                _ => self.open_files("Choose a background image", Filter::Images, FilePurpose::Background),
            },
        }
    }

    fn picker_cancelled(&mut self, kind: PickerKind) {
        // During onboarding, B skips to the next step.
        if let PickerKind::Accent { onboarding: true } = kind {
            self.open_picker(PickerKind::Avatar { onboarding: true });
        }
    }

    fn open_keyboard(&mut self, title: &str, initial: &str, max_len: usize, purpose: OskPurpose) {
        let mut osk = Osk::new(title, initial, max_len);
        osk.placeholder = match purpose {
            OskPurpose::CustomAccent { .. } => "#107C10".into(),
            OskPurpose::NewProfile | OskPurpose::RenameProfile => "Profile name".into(),
            OskPurpose::NewGameName { .. } => "Game name".into(),
            OskPurpose::AppTitle => "Win2Console".into(),
            OskPurpose::AddFriend => "Username".into(),
            OskPurpose::Bio => "Say something about yourself".into(),
        };
        self.modal = Some(ModalState::Keyboard { osk, purpose });
    }

    /// Called after typing changes the text of the active keyboard.
    fn text_changed(&mut self) {
        if self.view == View::Search && self.modal.is_none() {
            self.update_search();
        }
        if let Some(si) = &self.signin
            && self.view == View::SignIn
        {
            si.web.set_value(&si.osk.text());
        }
        self.render();
    }

    fn osk_outcome(&mut self, outcome: OskOutcome) {
        // Chat message being written in the Guide.
        if matches!(self.modal, Some(ModalState::Guide { .. }))
            && let Some(chat) = &mut self.guide_chat
            && chat.compose.is_some()
        {
            match outcome {
                OskOutcome::Continue => {}
                OskOutcome::Cancel => chat.compose = None,
                OskOutcome::Done(text) => {
                    let peer = chat.peer;
                    chat.compose = None;
                    if !text.is_empty()
                        && let Some(s) = &mut self.social
                    {
                        s.send_message(peer, &text);
                    }
                }
            }
            return;
        }
        // Sign-in page keyboard: text goes straight into the page's field.
        if self.modal.is_none() && self.view == View::SignIn {
            let Some(si) = &mut self.signin else { return };
            match outcome {
                OskOutcome::Continue => si.web.set_value(&si.osk.text()),
                OskOutcome::Cancel => si.zone = SignInZone::Page,
                OskOutcome::Done(_) => {
                    si.web.set_value(&si.osk.text());
                    si.zone = SignInZone::Page;
                    si.web.nav("down");
                }
            }
            return;
        }
        // Search's docked keyboard.
        if self.modal.is_none() && self.view == View::Search {
            match outcome {
                OskOutcome::Continue => self.update_search(),
                OskOutcome::Cancel => {
                    let ret = self.search.take().map_or(View::Home, |s| s.ret);
                    self.go(ret);
                }
                OskOutcome::Done(_) => {
                    if let Some(s) = &mut self.search {
                        s.focus = (!s.results.is_empty()).then_some(0);
                    }
                }
            }
            return;
        }
        let purpose = match &self.modal {
            Some(ModalState::Keyboard { purpose, .. }) => purpose.clone(),
            _ => return,
        };
        match outcome {
            OskOutcome::Continue => {}
            OskOutcome::Cancel => {
                self.modal = None;
                if let OskPurpose::CustomAccent { onboarding: true } = purpose {
                    self.open_picker(PickerKind::Avatar { onboarding: true });
                }
            }
            OskOutcome::Done(text) => {
                self.modal = None;
                self.osk_done(purpose, text);
            }
        }
    }

    fn osk_done(&mut self, purpose: OskPurpose, text: String) {
        match purpose {
            OskPurpose::Bio => self.update_profile(|p| p.bio = text.trim().to_string()),
            OskPurpose::AddFriend => {
                if !text.trim().is_empty()
                    && let Some(s) = &mut self.social
                {
                    s.send_friend_request(&text);
                }
            }
            OskPurpose::AppTitle => {
                let t = text.trim();
                self.config.app_title = (!t.is_empty()).then(|| t.to_string());
                self.save_config();
                self.apply_branding();
            }
            OskPurpose::NewProfile => match self.store.create(&text) {
                Ok(p) => {
                    self.profiles = self.store.list().unwrap_or_default();
                    let index = self.profiles.iter().position(|q| q.id == p.id).unwrap_or(0);
                    self.sign_in(index);
                    self.open_picker(PickerKind::Accent { onboarding: true });
                }
                Err(e) => self.toast(format!("Couldn't create the profile: {e}")),
            },
            OskPurpose::RenameProfile => {
                if text.is_empty() {
                    self.toast("The name can't be empty");
                } else {
                    self.update_profile(|p| p.name = text);
                }
            }
            OskPurpose::CustomAccent { onboarding } => {
                let hex = if text.starts_with('#') { text } else { format!("#{text}") };
                match cs_core::model::parse_accent(&hex) {
                    Some(hex) => self.update_profile(|p| p.accent = hex),
                    None => self.toast("Use six hex digits, like #107C10"),
                }
                if onboarding {
                    self.open_picker(PickerKind::Avatar { onboarding: true });
                }
            }
            OskPurpose::NewGameName { exe } => {
                let name = if text.is_empty() { cs_library::custom::default_name(&exe) } else { text };
                let game = cs_library::custom::new_custom_game(&name, exe, Vec::new(), None);
                let Some(profile) = self.profile_id() else { return };
                if let Err(e) = self.library.upsert_game(profile, &game) {
                    log_db_err::<()>(e);
                    self.toast("Couldn't add the game");
                    return;
                }
                self.toast(format!("{name} added to My games & apps"));
                self.lib_category = 0;
                self.lib_zone = LibZone::Grid;
                self.go(View::Library);
                self.lib_focus = self.lib_games.iter().position(|e| e.game.id == game.id).unwrap_or(0);
                self.open_menu(
                    &format!("Add art for {name}?"),
                    "You can change it later from the game's options (Menu button).",
                    vec![
                        entry("\u{EB9F}", "Choose cover art", MenuAction::SetArt(game.id.clone(), ArtSlot::Cover)),
                        entry("\u{E711}", "Not now", MenuAction::Close),
                    ],
                );
            }
        }
    }

    // ================================================================ actions

    fn update_profile(&mut self, f: impl FnOnce(&mut Profile)) {
        let Some(mut p) = self.current.clone() else { return };
        f(&mut p);
        match self.store.save(&p) {
            Ok(()) => {
                if let Some(slot) = self.profiles.iter_mut().find(|q| q.id == p.id) {
                    *slot = p.clone();
                }
                self.current = Some(p);
            }
            Err(e) => self.toast(format!("Couldn't save the profile: {e}")),
        }
    }

    fn set_custom_avatar(&mut self, path: &Path) {
        let Some(id) = self.profile_id() else { return };
        let result = images::avatar_png(path)
            .map_err(|e| e.to_string())
            .and_then(|png| self.store.write_avatar_png(id, &png).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                self.images.invalidate(&self.store.avatar_path(id));
                self.update_profile(|p| p.avatar = Avatar::Custom);
            }
            Err(e) => self.toast(format!("Couldn't use that picture: {e}")),
        }
    }

    fn set_custom_background(&mut self, path: &Path) {
        let Some(id) = self.profile_id() else { return };
        let result = images::png_fit(path, images::STORED_BACKGROUND_MAX)
            .map_err(|e| e.to_string())
            .and_then(|png| self.store.write_background_png(id, &png).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                self.images.invalidate(&self.store.background_path(id));
                self.update_profile(|p| p.background = Background::Custom);
            }
            Err(e) => self.toast(format!("Couldn't use that image: {e}")),
        }
    }

    fn set_art(&mut self, game_id: &str, slot: ArtSlot, src: &Path) {
        let (file, max) = match slot {
            ArtSlot::Cover => ("cover.png", images::STORED_COVER_MAX),
            ArtSlot::Background => ("background.png", images::STORED_BACKGROUND_MAX),
        };
        let dst = self.paths.game_media_dir(game_id).join(file);
        if let Err(e) = images::store_image(src, &dst, max) {
            self.toast(format!("Couldn't use that image: {e}"));
            return;
        }
        self.images.invalidate(&dst);
        let Ok(Some(mut game)) = self.library.game(game_id) else { return };
        match slot {
            ArtSlot::Cover => game.media.cover = Some(dst),
            ArtSlot::Background => game.media.background = Some(dst),
        }
        if let Some(profile) = self.profile_id()
            && let Err(e) = self.library.upsert_game(profile, &game)
        {
            log_db_err::<()>(e);
        }
        self.reload();
        if self.view == View::Library {
            self.lib_focus = self.lib_games.iter().position(|e| e.game.id == game_id).unwrap_or(self.lib_focus);
        }
        self.toast("Artwork updated");
    }

    fn toggle_pin(&mut self, game_id: &str) {
        let Some(p) = self.profile_id() else { return };
        let pinned = self.library.pinned(p).unwrap_or_default().iter().any(|e| e.game.id == game_id);
        if let Err(e) = self.library.set_pinned(p, game_id, !pinned) {
            log_db_err::<()>(e);
        }
        self.toast(if pinned { "Unpinned from Home" } else { "Pinned to Home" });
        self.reload();
    }

    fn toggle_hidden(&mut self, game_id: &str) {
        let Some(p) = self.profile_id() else { return };
        let hidden =
            self.library.list(p, true).unwrap_or_default().iter().any(|e| e.game.id == game_id && e.stats.hidden);
        if let Err(e) = self.library.set_hidden(p, game_id, !hidden) {
            log_db_err::<()>(e);
        }
        self.toast(if hidden { "Game unhidden" } else { "Game hidden. Find it under Hidden in My games & apps." });
        self.reload();
    }

    fn delete_current_profile(&mut self) {
        let Some(p) = self.current.clone() else { return };
        if let Err(e) = self.library.delete_profile_stats(p.id) {
            log_db_err::<()>(e);
        }
        if let Err(e) = self.store.delete(p.id) {
            self.toast(format!("Couldn't delete the profile: {e}"));
            return;
        }
        if self.config.last_profile == Some(p.id) {
            self.config.last_profile = None;
            self.save_config();
        }
        self.switch_profile();
        self.profiles_focus = 0;
        self.toast(format!("Deleted {}", p.name));
    }

    fn launch(&mut self, game: &Game) {
        crate::sounds::play(crate::sounds::Sound::Launch);
        let wanted = (game.provider == Provider::Steam)
            .then(|| self.current.as_ref()?.accounts.iter().find(|a| a.service == Service::Steam).cloned())
            .flatten();
        match wanted {
            Some(a) if cs_library::steam::active_user().as_deref() != Some(a.account_id.as_str()) => {
                self.show_launch_screen(game);
                self.ui().set_launch_status(format!("Switching Steam to {}…", a.display_name).into());
                let game = game.clone();
                std::thread::spawn(move || {
                    let result = cs_library::steam::switch_user(&a.account_id);
                    post(move |app| match result {
                        Ok(()) => app.launch_now(&game),
                        Err(e) => {
                            app.close_launch_screen();
                            app.toast(format!("Couldn't switch Steam accounts: {e}"));
                        }
                    });
                });
            }
            _ => self.launch_now(game),
        }
    }

    fn launch_now(&mut self, game: &Game) {
        let Some(profile) = self.profile_id() else { return };
        match cs_platform::process::launch(&game.launch) {
            Ok(pid) => {
                self.show_launch_screen(game);
                self.stop_tracking();
                let game_id = game.id.clone();
                let handle = tracker::spawn(game, pid, Arc::new(SystemProcesses), {
                    let game_id = game_id.clone();
                    move |ev| {
                        let game_id = game_id.clone();
                        post(move |app| app.tracker_event(profile, &game_id, ev));
                    }
                });
                self.running = Some(Running { game_id, profile, handle });
                if let Some(s) = &mut self.social {
                    s.set_playing(Some(&game.name));
                }
            }
            Err(e) => {
                self.close_launch_screen();
                self.toast(format!("Couldn't start {}: {e}", game.name))
            }
        }
    }

    fn tracker_event(&mut self, profile: uuid::Uuid, game_id: &str, ev: TrackerEvent) {
        let now = cs_core::now_unix();
        let secs = match ev {
            TrackerEvent::Started => {
                if let Some(l) = &mut self.launching {
                    l.started = Some(Instant::now());
                }
                0
            }
            TrackerEvent::Ended { secs } | TrackerEvent::Cancelled { secs } => secs,
            TrackerEvent::NotDetected => {
                self.close_launch_screen();
                self.toast("Couldn't detect the game running, so playtime won't be recorded");
                self.running = None;
                self.render();
                return;
            }
        };
        if let Err(e) = self.library.record_session(profile, game_id, secs, now) {
            log_db_err::<()>(e);
        }
        if matches!(ev, TrackerEvent::Ended { .. }) {
            self.running = None;
            if let Some(s) = &mut self.social {
                s.set_playing(None);
            }
            if self.ingame.take().is_some() {
                self.modal = None;
            }
            // Like a console: when the game closes, come back to the dashboard.
            cs_platform::window::bring_own_window_to_front();
        }
        self.reload();
        self.render();
    }

    fn note_transitions(&mut self) {
        let guide = std::mem::discriminant(&ModalState::Guide { tab: 0, focus: 0 });
        let modal = self.modal.as_ref().map(|m| match m {
            ModalState::Friend { .. } | ModalState::Chats { .. } => guide,
            m => std::mem::discriminant(m),
        });
        let now = Instant::now();
        // The splash and profile picker have their own entrances; animate dashboard page changes.
        if self.view != self.last_view {
            if self.last_view != View::Splash {
                self.view_changed = Some(now);
                self.ui().set_view_t(0.0);
            }
            self.last_view = self.view;
        }
        if modal != self.last_modal {
            if modal.is_some() {
                self.modal_changed = Some(now);
                self.ui().set_modal_t(0.0);
            }
            self.last_modal = modal;
        }
        // Home: sliding between sections.
        if self.view == View::Home && self.home_section != self.last_home_section {
            let dir = if self.home_section > self.last_home_section { 1 } else { -1 };
            self.last_home_section = self.home_section;
            self.section_changed = Some(now);
            self.ui().set_section_dir(dir);
            self.ui().set_section_t(0.0);
        }
        if self.view_changed.is_some() || self.modal_changed.is_some() || self.section_changed.is_some() {
            self.start_anim();
        }
    }

    fn start_anim(&mut self) {
        if !self.anim_timer.running() {
            self.anim_timer.start(slint::TimerMode::Repeated, Duration::from_millis(16), || {
                crate::with_app(|app| app.anim_tick());
            });
        }
    }

    fn anim_tick(&mut self) {
        let ui = self.ui();
        let now = Instant::now();
        if let Some(t) = self.view_changed {
            let p = now.duration_since(t).as_secs_f32() / VIEW_ANIM;
            ui.set_view_t(ease_out(p));
            if p >= 1.0 {
                self.view_changed = None;
            }
        }
        if let Some(t) = self.section_changed {
            let p = now.duration_since(t).as_secs_f32() / SECTION_ANIM;
            ui.set_section_t(ease_out(p));
            if p >= 1.0 {
                self.section_changed = None;
            }
        }
        if let Some(t) = self.modal_changed {
            let p = now.duration_since(t).as_secs_f32() / MODAL_ANIM;
            ui.set_modal_t(ease_out(p));
            if p >= 1.0 {
                self.modal_changed = None;
            }
        }
        if let Some(l) = &mut self.launching {
            let shown = now.duration_since(l.shown).as_secs_f32();
            ui.set_launch_t(shown);
            match l.closing {
                Some(c) => {
                    let p = now.duration_since(c).as_secs_f32() / LAUNCH_FADE;
                    ui.set_launch_fade(1.0 - ease_out(p));
                    if p >= 1.0 {
                        self.launching = None;
                        ui.set_launching(false);
                        self.render();
                    }
                }
                None => {
                    ui.set_launch_fade(ease_out(shown / LAUNCH_FADE));
                    // Once the game is running, switch to its window as soon as it has one.
                    if let Some(started) = l.started
                        && shown >= LAUNCH_MIN
                        && now.duration_since(l.last_focus_try) >= Duration::from_millis(400)
                    {
                        l.last_focus_try = now;
                        let waited = now.duration_since(started);
                        let pids = self.game_pids();
                        if cs_platform::window::focus_process(&pids) || waited > Duration::from_secs(60) {
                            if let Some(l) = &mut self.launching {
                                l.closing = Some(now);
                            }
                        }
                    }
                }
            }
        }
        if self.view_changed.is_none()
            && self.modal_changed.is_none()
            && self.section_changed.is_none()
            && self.launching.is_none()
        {
            self.anim_timer.stop();
        }
    }

    fn show_launch_screen(&mut self, game: &Game) {
        let ui = self.ui();
        let media = &game.media;
        let cover = media.cover.as_ref().and_then(|p| self.images.get(p, Size::Tile));
        let art = media.background.as_ref().or(media.cover.as_ref()).and_then(|p| self.images.get(p, Size::Backdrop));
        ui.set_launch_cover(cover.unwrap_or_default());
        ui.set_launch_art(art.unwrap_or_default());
        ui.set_launch_title(game.name.clone().into());
        ui.set_launch_status("Starting…".into());
        ui.set_launch_fade(0.0);
        ui.set_launching(true);
        let now = Instant::now();
        self.launching = Some(Launching { shown: now, started: None, closing: None, last_focus_try: now });
        self.start_anim();
    }

    fn close_launch_screen(&mut self) {
        if let Some(l) = &mut self.launching
            && l.closing.is_none()
        {
            l.closing = Some(Instant::now());
            self.start_anim();
        }
    }

    /// Saves the in-game Guide's capture to `Pictures\Win2Console` (encoded on a background thread).
    fn save_screenshot(&mut self) {
        let Some(capture) = self.ingame.as_ref().and_then(|g| g.capture.clone()) else { return };
        let game = self.running_entry().map(|e| e.game.name).unwrap_or_else(|| "Screenshot".into());
        let safe: String = game.chars().filter(|c| !r#"\/:*?"<>|"#.contains(*c)).collect();
        let Some(dir) = Self::captures_dir() else { return };
        let path = dir.join(format!("{} {}.png", safe.trim(), cs_platform::time::file_timestamp()));
        std::thread::spawn(move || {
            let result = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|()| {
                image::RgbaImage::from_raw(capture.width, capture.height, capture.rgba.clone())
                    .ok_or_else(|| "bad capture".to_string())?
                    .save(&path)
                    .map_err(|e| e.to_string())
            });
            post(move |app| {
                match result {
                    Ok(()) => app.toast("Screenshot saved to Pictures\\Win2Console"),
                    Err(e) => app.toast(format!("Couldn't save the screenshot: {e}")),
                }
                app.render();
            });
        });
    }

    fn open_game_page(&mut self, id: &str) {
        let Some(e) = self.entry_by_id(id) else { return };
        let achievements =
            cs_library::achievements::Achievements::open().map(|a| a.list(&game_key(&e.game))).unwrap_or_default();
        let ret = if self.view == View::Game { View::Home } else { self.view };
        self.game_page = Some(GamePageState { game_id: id.to_string(), focus: 0, achievements, ret });
        self.view = View::Game;
    }

    fn nav_game(&mut self, action: NavAction) {
        let Some(g) = &mut self.game_page else { return };
        let total = GAME_BUTTONS + g.achievements.len();
        match action {
            NavAction::Left if g.focus > 0 && g.focus < GAME_BUTTONS => g.focus -= 1,
            NavAction::Right if g.focus + 1 < GAME_BUTTONS => g.focus += 1,
            NavAction::Right if g.focus + 1 == GAME_BUTTONS && total > GAME_BUTTONS => g.focus = GAME_BUTTONS,
            NavAction::Left if g.focus >= GAME_BUTTONS => g.focus = GAME_BUTTONS - 1,
            NavAction::Down if g.focus < GAME_BUTTONS && total > GAME_BUTTONS => g.focus = GAME_BUTTONS,
            NavAction::Down if g.focus >= GAME_BUTTONS && g.focus + 1 < total => g.focus += 1,
            NavAction::Up if g.focus > GAME_BUTTONS => g.focus -= 1,
            NavAction::Up if g.focus == GAME_BUTTONS => g.focus = 0,
            NavAction::Back => {
                let ret = g.ret;
                self.game_page = None;
                self.go(ret);
            }
            NavAction::Menu | NavAction::Accept if g.focus < GAME_BUTTONS => {
                let (focus, id) = (if action == NavAction::Menu { 2 } else { g.focus }, g.game_id.clone());
                let Some(e) = self.entry_by_id(&id) else { return };
                match focus {
                    0 => self.play(&e),
                    1 => self.toggle_pin(&id),
                    _ => self.open_game_menu(&e),
                }
            }
            _ => {}
        }
    }

    fn render_game(&mut self, ui: &AppWindow) {
        let Some(g) = &self.game_page else { return };
        let (focus, list) = (g.focus, g.achievements.clone());
        let Some(e) = self.entry_by_id(&g.game_id.clone()) else { return };
        let media = &e.game.media;
        let cover = media.cover.as_ref().and_then(|p| self.images.get(p, Size::Tile));
        let art = media.background.as_ref().or(media.cover.as_ref()).and_then(|p| self.images.get(p, Size::Backdrop));
        ui.set_game_title(e.game.name.clone().into());
        let hours = e.stats.playtime_secs as f64 / 3600.0;
        let played = if e.stats.playtime_secs > 0 { format!(" · {hours:.1} h played") } else { String::new() };
        ui.set_game_subtitle(format!("{}{played}", e.game.provider.display_name()).into());
        let s = cs_library::achievements::summarize(&list);
        ui.set_game_stats(
            if list.is_empty() {
                String::new()
            } else {
                format!("{}/{} achievements · {}/{} G", s.unlocked, s.total, s.gamerscore, s.max_gamerscore)
            }
            .into(),
        );
        ui.set_game_has_cover(cover.is_some());
        ui.set_game_cover(cover.unwrap_or_default());
        ui.set_game_art(art.unwrap_or_default());
        let buttons = [
            (if e.installed { "\u{E768}" } else { "\u{E896}" }, if e.installed { "Play" } else { "Install" }),
            ("\u{E718}", if e.stats.pinned { "Unpin" } else { "Pin to Home" }),
            ("\u{E712}", "More options"),
        ]
        .map(|(g, l)| GameButton { glyph: g.into(), label: l.into() });
        ui.set_game_buttons(Rc::new(VecModel::from(buttons.to_vec())).into());
        // Only load icons near the selection (big achievement lists).
        let sel = focus.saturating_sub(GAME_BUTTONS);
        let rows: Vec<AchievementRow> = list
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let near = i + 4 >= sel && i <= sel + 12;
                let icon = a
                    .icon
                    .as_ref()
                    .and_then(|p| if near { self.images.get(p, Size::Tile) } else { self.images.peek(p, Size::Tile) });
                AchievementRow {
                    name: a.name.clone().into(),
                    description: a.description.clone().into(),
                    has_icon: icon.is_some(),
                    icon: icon.unwrap_or_default(),
                    unlocked: a.unlocked,
                    detail: if a.unlocked {
                        format!("{} G", a.gamerscore)
                    } else {
                        a.rarity
                            .map_or_else(|| format!("{} G", a.gamerscore), |r| format!("{} G · {r:.1}%", a.gamerscore))
                    }
                    .into(),
                }
            })
            .collect();
        ui.set_game_achievements(Rc::new(VecModel::from(rows)).into());
        ui.set_game_empty(
            "No achievements yet. Install the Playnite Achievements add-on in Playnite and let it sync.".into(),
        );
        ui.set_game_focus(focus as i32);
    }

    fn open_my_profile(&mut self) {
        let ret = if self.view == View::MyProfile { View::Home } else { self.view };
        self.my_profile = (0, ret);
        self.view = View::MyProfile;
    }

    /// The games on your profile page: the ones you picked, or else your most played.
    fn showcase(&self) -> Vec<GameEntry> {
        let (Some(p), Some(id)) = (&self.current, self.profile_id()) else { return Vec::new() };
        let all = self.library.list(id, true).unwrap_or_default();
        if !p.showcase.is_empty() {
            return p.showcase.iter().filter_map(|g| all.iter().find(|e| &e.game.id == g).cloned()).collect();
        }
        let mut played: Vec<GameEntry> = all.into_iter().filter(|e| e.stats.playtime_secs > 0).collect();
        played.sort_by(|a, b| b.stats.playtime_secs.cmp(&a.stats.playtime_secs));
        played.truncate(SHOWCASE_MAX);
        played
    }

    fn nav_my_profile(&mut self, action: NavAction) {
        let buttons = PROFILE_BUTTONS.len();
        let total = buttons + self.showcase().len();
        let (focus, ret) = self.my_profile;
        let focus = match action {
            NavAction::Up if focus > 0 && focus < buttons => focus - 1,
            NavAction::Up if focus >= buttons => buttons - 1,
            NavAction::Down if focus + 1 < buttons => focus + 1,
            NavAction::Down if focus + 1 == buttons && total > buttons => buttons,
            NavAction::Left if focus > buttons => focus - 1,
            NavAction::Right if focus >= buttons && focus + 1 < total => focus + 1,
            NavAction::Back => return self.go(ret),
            NavAction::Accept => {
                match focus {
                    0 => self.open_settings(0),
                    1 => {
                        let bio = self.current.as_ref().map(|p| p.bio.clone()).unwrap_or_default();
                        self.open_keyboard("Your bio", &bio, 150, OskPurpose::Bio);
                    }
                    2 => self.open_showcase_menu(None),
                    3 => self.share_profile(),
                    _ => {
                        if let Some(e) = self.showcase().get(focus - buttons) {
                            let id = e.game.id.clone();
                            self.open_game_page(&id);
                        }
                    }
                }
                return;
            }
            _ => focus,
        };
        self.my_profile.0 = focus;
    }

    /// Lists your games to pick the showcase from (most played first); `focus_id` keeps the selection.
    fn open_showcase_menu(&mut self, focus_id: Option<&str>) {
        let (Some(p), Some(id)) = (&self.current, self.profile_id()) else { return };
        let picked = p.showcase.clone();
        let mut all = self.library.list(id, true).unwrap_or_default();
        all.sort_by(|a, b| b.stats.playtime_secs.cmp(&a.stats.playtime_secs));
        let focus = focus_id.and_then(|f| all.iter().position(|e| e.game.id == f)).unwrap_or(0);
        let entries = all
            .iter()
            .map(|e| {
                let glyph = if picked.contains(&e.game.id) { "\u{E73A}" } else { "\u{E739}" };
                entry(glyph, &e.game.name, MenuAction::ToggleShowcase(e.game.id.clone()))
            })
            .collect();
        self.open_menu(
            "Showcase games",
            &format!("Pick up to {SHOWCASE_MAX} games for your profile. With none picked, your most played show."),
            entries,
        );
        if let Some(ModalState::Menu { focus: f, .. }) = &mut self.modal {
            *f = focus;
        }
    }

    fn toggle_showcase(&mut self, id: String) {
        let full =
            self.current.as_ref().is_some_and(|p| p.showcase.len() >= SHOWCASE_MAX && !p.showcase.contains(&id));
        if full {
            self.toast(format!("You can showcase up to {SHOWCASE_MAX} games"));
        } else {
            let game = id.clone();
            self.update_profile(move |p| {
                if let Some(i) = p.showcase.iter().position(|g| *g == game) {
                    p.showcase.remove(i);
                } else {
                    p.showcase.push(game);
                }
            });
        }
        self.open_showcase_menu(Some(&id));
    }

    /// Saves a picture of your profile page to Captures, ready to send to friends.
    fn share_profile(&mut self) {
        let ui = self.ui();
        ui.set_profile_sharing(true);
        let shot = ui.window().take_snapshot();
        ui.set_profile_sharing(false);
        let (Ok(shot), Some(dir)) = (shot, Self::captures_dir()) else {
            self.toast("Couldn't take a picture of your profile");
            return;
        };
        let name = self.current.as_ref().map_or_else(|| "Profile".to_string(), |p| p.name.clone());
        let safe: String = name.chars().filter(|c| !r#"\/:*?"<>|"#.contains(*c)).collect();
        let path = dir.join(format!("{} profile {}.png", safe.trim(), cs_platform::time::file_timestamp()));
        let saved = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| {
            image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec())
                .ok_or_else(|| "bad image".to_string())?
                .save(&path)
                .map_err(|e| e.to_string())
        });
        match saved {
            Ok(()) => {
                self.refresh_captures();
                self.toast("Saved a picture of your profile to Captures, ready to share");
            }
            Err(e) => self.toast(format!("Couldn't save the picture: {e}")),
        }
    }

    fn render_my_profile(&mut self, ui: &AppWindow) {
        let Some(p) = self.current.clone() else { return };
        let all = self.profile_id().map(|id| self.library.list(id, true).unwrap_or_default()).unwrap_or_default();
        let hours = all.iter().map(|e| e.stats.playtime_secs).sum::<i64>() as f64 / 3600.0;
        let stats = [
            ("Friends", self.friends.len().to_string()),
            ("Games", all.len().to_string()),
            ("Hours played", format!("{hours:.0}")),
        ]
        .map(|(l, v)| ProfileStat { label: l.into(), value: v.into() });
        ui.set_profile_stats(Rc::new(VecModel::from(stats.to_vec())).into());
        ui.set_profile_bio(p.bio.clone().into());
        let buttons = PROFILE_BUTTONS.map(|(g, l)| GameButton { glyph: g.into(), label: l.into() });
        ui.set_profile_buttons(Rc::new(VecModel::from(buttons.to_vec())).into());
        let games: Vec<Tile> = self
            .showcase()
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let mut t = self.game_tile(e, Rect::new(66.0 + 216.0 * i as f32, 700.0, 200.0, 200.0), false);
                t.caption = true;
                t
            })
            .collect();
        ui.set_profile_games(Rc::new(VecModel::from(games)).into());
        ui.set_profile_focus(self.my_profile.0 as i32);
    }

    /// Loads the total Gamerscore in the background (the add-on's database can be large).
    fn refresh_gamerscore(&mut self) {
        std::thread::spawn(|| {
            let total = cs_library::achievements::Achievements::open().map(|a| a.total_gamerscore());
            post(move |app| {
                app.gamerscore = total;
                app.render();
            });
        });
    }

    /// Window title and icon from the settings.
    pub fn apply_branding(&mut self) {
        let ui = self.ui();
        ui.set_menu_scale(self.config.ui_scale.unwrap_or(100) as f32 / 100.0);
        ui.set_app_title(self.config.app_title.clone().unwrap_or_else(|| "Win2Console".into()).into());
        let rgba = image::open(self.paths.root.join(APP_ICON)).ok().map(|img| img.into_rgba8());
        let icon = rgba.as_ref().map(|rgba| {
            slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
            ))
        });
        ui.set_app_icon(icon.unwrap_or_default());
        // Slint only sets the small (title bar) icon; the taskbar and Alt+Tab use the big one.
        #[cfg(windows)]
        {
            use slint::winit_030::WinitWindowAccessor;
            use slint::winit_030::winit::platform::windows::WindowExtWindows;
            use slint::winit_030::winit::window::Icon;
            let big = rgba.and_then(|r| {
                let (w, h) = r.dimensions();
                Icon::from_rgba(r.into_raw(), w, h).ok()
            });
            // At startup the native window may not exist yet: try again shortly.
            if ui.window().with_winit_window(|w| w.set_taskbar_icon(big)).is_none() {
                slint::Timer::single_shot(Duration::from_millis(250), || {
                    crate::with_app(|app| app.apply_branding());
                });
            }
        }
    }

    fn load_avatar_pack(&mut self) {
        let dir = self.paths.root.join("avatars");
        let _ = std::fs::create_dir_all(&dir); // so it's easy to find and fill
        let mut pack: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| ["png", "jpg", "jpeg", "webp", "bmp"].contains(&x.to_ascii_lowercase().as_str()))
            })
            .collect();
        pack.sort();
        pack.truncate(21); // 5 rows of 6 with the built-ins
        self.avatar_pack = pack;
    }

    pub fn set_cursor_hidden(&mut self, hidden: bool) {
        if self.cursor_hidden == hidden {
            return;
        }
        self.cursor_hidden = hidden;
        use slint::winit_030::WinitWindowAccessor;
        self.ui().window().with_winit_window(|w| w.set_cursor_visible(!hidden));
    }

    /// Shows `message` in the always-on-top notification window at the bottom centre of the screen.
    fn notify_over_game(&mut self, message: &str) {
        self.notify_over_game_with(message, None);
    }

    /// A notification over the game, with a picture (a friend's) in place of the bell when there is one.
    fn notify_over_game_with(&mut self, message: &str, picture: Option<Image>) {
        use slint::ComponentHandle;
        if self.notify.is_none() {
            match crate::ui::NotifyWindow::new() {
                Ok(w) => self.notify = Some(w),
                Err(e) => return tracing::warn!("notification window: {e}"),
            }
        }
        let Some(w) = &self.notify else { return };
        let accent = self.current.as_ref().map_or(cs_core::model::DEFAULT_ACCENT, |p| p.accent.as_str());
        w.set_accent(parse_color(accent));
        w.set_message(message.into());
        w.set_has_picture(picture.is_some());
        w.set_picture(picture.unwrap_or_default());
        let (sw, sh) = cs_platform::window::screen_size();
        let (ww, wh) = ((sw as f32 * 0.3) as u32, (sh as f32 * 0.085) as u32);
        w.window().set_size(slint::PhysicalSize::new(ww, wh));
        w.window().set_position(slint::PhysicalPosition::new((sw - ww as i32) / 2, sh - wh as i32 - sh / 12));
        if let Err(e) = w.show() {
            return tracing::warn!("notification window: {e}");
        }
        // Never take focus from the game or catch its clicks.
        if let Some(hwnd) = overlay_hwnd(w.window()) {
            cs_platform::window::make_overlay(hwnd);
        }
        self.notify_timer.start(slint::TimerMode::SingleShot, Duration::from_secs(4), || {
            crate::with_app(|app| {
                if let Some(w) = &app.notify {
                    let _ = w.hide();
                }
            });
        });
    }

    /// At startup: automatic mode downloads any update (installed when Win2Console closes); manual mode
    /// only says one is available.
    pub fn startup_update_check(&mut self) {
        self.check_updates(!self.config.manual_updates, false);
    }

    fn check_updates(&mut self, download: bool, announce_result: bool) {
        if matches!(self.update_state, updater::State::Checking | updater::State::Downloading(_)) {
            return;
        }
        self.update_state = updater::State::Checking;
        std::thread::spawn(move || {
            let (state, info) = updater::check(download);
            post(move |app| {
                match &state {
                    updater::State::Available(v) => {
                        app.toast(format!("Win2Console {v} is available. Install it from Settings → System → Updates."))
                    }
                    updater::State::Ready(v) => {
                        app.toast(format!("Win2Console {v} is ready and installs when you close Win2Console"))
                    }
                    other if announce_result => app.toast(other.label()),
                    _ => {}
                }
                if let updater::State::Failed(e) = &state {
                    tracing::warn!("update check: {e}");
                }
                app.update_state = state;
                app.update_info = info;
                app.render();
            });
        });
    }

    fn captures_dir() -> Option<PathBuf> {
        let pictures = std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Pictures"))?;
        let dir = pictures.join("Win2Console");
        cs_core::paths::adopt_old_dir(&pictures.join("ConsoleShell"), &dir);
        Some(dir)
    }

    fn refresh_captures(&mut self) {
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = Self::captures_dir()
            .and_then(|d| std::fs::read_dir(d).ok())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("png") || x.eq_ignore_ascii_case("mp4"))
            })
            .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path()))
            .collect();
        files.sort_by(|a, b| b.0.cmp(&a.0));
        self.captures = files.into_iter().map(|(_, p)| p).take(30).collect();
        self.captures_at = Some(Instant::now());
    }

    fn load_ach_guide(&mut self) {
        let Some(db) = cs_library::achievements::Achievements::open() else {
            self.ach_guide = Some(AchGuide { at: Instant::now(), games: Vec::new(), recent: Vec::new() });
            return;
        };
        let recent_games = self.profile_id().map(|p| self.library.recent(p, 5).unwrap_or_default()).unwrap_or_default();
        let games = recent_games.into_iter().filter_map(|e| db.summary(&game_key(&e.game)).map(|s| (e, s))).collect();
        let all = self.profile_id().map(|p| self.library.list(p, true).unwrap_or_default()).unwrap_or_default();
        let recent = db
            .recent_unlocks(8)
            .into_iter()
            .map(|r| {
                let id = all
                    .iter()
                    .find(|e| {
                        let k = game_key(&e.game);
                        (r.playnite_id.is_some() && k.playnite_id == r.playnite_id.as_deref())
                            || (r.provider_id.is_some() && k.provider_id == r.provider_id)
                            || e.game.name.eq_ignore_ascii_case(&r.game)
                    })
                    .map(|e| e.game.id.clone());
                (id, r)
            })
            .collect();
        self.ach_guide = Some(AchGuide { at: Instant::now(), games, recent });
    }

    fn start_recording(&mut self) {
        let Some(ffmpeg) = find_ffmpeg() else {
            self.toast("Recording needs ffmpeg (install it with: winget install Gyan.FFmpeg)");
            return;
        };
        let Some(dir) = Self::captures_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let game = self.running_entry().map(|e| e.game.name).unwrap_or_else(|| "Recording".into());
        let safe: String = game.chars().filter(|c| !r#"\/:*?"<>|"#.contains(*c)).collect();
        let path = dir.join(format!("{} {}.mp4", safe.trim(), cs_platform::time::file_timestamp()));
        let mut cmd = std::process::Command::new(ffmpeg);
        cmd.args(["-y", "-loglevel", "error", "-f", "gdigrab", "-framerate", "30", "-i", "desktop"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "23", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        match cmd.spawn() {
            Ok(child) => {
                self.recording = Some(Recording { child, started: Instant::now(), path });
                self.toast("Recording started. Open the Guide to stop.");
                if self.ingame.is_some() {
                    self.close_guide();
                }
            }
            Err(e) => self.toast(format!("Couldn't start recording: {e}")),
        }
    }

    fn stop_recording(&mut self) {
        let Some(mut r) = self.recording.take() else { return };
        // ffmpeg finishes the file cleanly when it reads "q".
        if let Some(mut stdin) = r.child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(b"q\n");
        }
        let path = r.path;
        std::thread::spawn(move || {
            let ok = r.child.wait().is_ok() && path.is_file();
            post(move |app| {
                app.captures_at = None;
                app.toast(if ok {
                    "Recording saved to Pictures\\Win2Console"
                } else {
                    "The recording couldn't be saved"
                });
                app.render();
            });
        });
    }

    fn open_captures(&mut self, focus: usize) {
        self.refresh_captures();
        if self.captures.is_empty() {
            self.toast("No captures yet. Take a screenshot from the Guide while playing.");
            return;
        }
        let focus = focus.min(self.captures.len() - 1);
        self.modal = Some(ModalState::Picker { kind: PickerKind::Captures { count: self.captures.len() }, focus });
    }

    fn nav_viewer(&mut self, action: NavAction) {
        let Some(ModalState::Viewer { index }) = &mut self.modal else { return };
        let (i, n) = (*index, self.captures.len());
        match action {
            NavAction::Left if i > 0 => *index -= 1,
            NavAction::Right if i + 1 < n => *index += 1,
            NavAction::X => {
                let path = self.captures[i].clone();
                match std::fs::remove_file(&path) {
                    Ok(()) => {
                        self.images.invalidate(&path);
                        self.toast("Capture deleted");
                        self.open_captures(i);
                    }
                    Err(e) => self.toast(format!("Couldn't delete it: {e}")),
                }
            }
            NavAction::Back => self.open_captures(i),
            _ => {}
        }
    }

    fn open_audio(&mut self) {
        let game_pids = self.game_pids();
        let mut sources = Vec::new();
        match cs_platform::audio::master() {
            Ok((level, muted)) => sources.push(AudioSource { kind: AudioKind::Master, level, muted, value: None }),
            Err(e) => tracing::warn!("reading the volume: {e}"),
        }
        if let Ok(Some((level, muted))) = cs_platform::audio::app(&game_pids) {
            sources.push(AudioSource { kind: AudioKind::Game, level, muted, value: None });
        }
        let deafened = self.party.as_ref().map(|p| p.self_deafened);
        if let (Some(muted), Some(level)) = (deafened, self.social.as_mut().and_then(|s| s.output_volume())) {
            sources.push(AudioSource { kind: AudioKind::Party, level, muted, value: None });
        }
        // Party chat audio (Discord).
        if let Some(a) =
            self.discord_audio.clone().filter(|_| matches!(self.social_status, social::Status::Connected { .. }))
        {
            let self_muted = self.party.as_ref().is_some_and(|p| p.self_muted);
            sources.push(AudioSource {
                kind: AudioKind::MicVolume,
                level: a.mic_volume,
                muted: self_muted,
                value: None,
            });
            let ns = if a.noise_suppression { "On" } else { "Off" };
            sources.push(AudioSource {
                kind: AudioKind::NoiseSuppression,
                level: 0.0,
                muted: false,
                value: Some(ns.into()),
            });
            let mic = social::AudioInfo::device_name(&a.inputs, &a.input);
            sources.push(AudioSource { kind: AudioKind::Microphone, level: 0.0, muted: false, value: Some(mic) });
            let out = social::AudioInfo::device_name(&a.outputs, &a.output);
            sources.push(AudioSource { kind: AudioKind::Speakers, level: 0.0, muted: false, value: Some(out) });
        }
        if sources.is_empty() {
            self.toast("Couldn't read the volume");
            return;
        }
        self.modal = Some(ModalState::Audio { focus: 0, sources, game_pids });
    }

    fn nav_audio(&mut self, action: NavAction) {
        let Some(ModalState::Audio { focus, sources, game_pids }) = &mut self.modal else { return };
        let pids = game_pids.clone();
        let mut changed = None;
        match action {
            NavAction::Up => *focus = list_move(*focus, sources.len(), -1),
            NavAction::Down => *focus = list_move(*focus, sources.len(), 1),
            NavAction::Left | NavAction::Right if sources[*focus].value.is_none() => {
                let s = &mut sources[*focus];
                let step = if action == NavAction::Right { VOLUME_STEP } else { -VOLUME_STEP };
                s.level = ((s.level + step) * 20.0).round().clamp(0.0, 20.0) / 20.0;
                changed = Some((s.clone(), false));
            }
            NavAction::Accept => {
                let s = &mut sources[*focus];
                if s.value.is_none() {
                    s.muted = !s.muted;
                }
                changed = Some((s.clone(), true));
            }
            NavAction::Back => {
                let entries = self.guide_entries(0);
                let audio_row =
                    entries.iter().position(|e| e.action == GuideAction::Audio).unwrap_or(entries.len() + 3);
                self.modal = Some(ModalState::Guide { tab: 0, focus: audio_row });
                return;
            }
            _ => {}
        }
        let Some((s, mute_changed)) = changed else { return };
        let result = match s.kind {
            AudioKind::Master => cs_platform::audio::set_master(s.level, s.muted),
            AudioKind::Game => cs_platform::audio::set_app(&pids, s.level, s.muted),
            AudioKind::Party => {
                if let Some(social) = &mut self.social {
                    if mute_changed {
                        social.set_deafened(s.muted);
                    } else {
                        social.set_output_volume(s.level);
                    }
                }
                Ok(())
            }
            AudioKind::MicVolume => {
                if let Some(social) = &mut self.social {
                    if mute_changed {
                        social.set_muted(s.muted);
                    } else {
                        social.set_mic_volume(s.level);
                    }
                }
                Ok(())
            }
            AudioKind::NoiseSuppression => {
                let on = self.discord_audio.as_ref().is_some_and(|a| a.noise_suppression);
                if let Some(social) = &mut self.social {
                    social.set_noise_suppression(!on);
                }
                Ok(())
            }
            AudioKind::Microphone | AudioKind::Speakers => {
                if let Some(social) = &mut self.social {
                    social.next_audio_device(s.kind == AudioKind::Microphone);
                }
                Ok(())
            }
        };
        if let Err(e) = result {
            tracing::warn!("changing the volume: {e}");
        }
    }

    fn quit_game(&mut self) {
        let pids = self.game_pids();
        tracing::info!("quitting the game (pids {pids:?})");
        let name = self.running_entry().map(|e| e.game.name);
        self.leave_ingame();
        if pids.is_empty() {
            self.toast("Couldn't find the game to quit");
            return;
        }
        for pid in pids {
            if let Err(e) = cs_platform::process::terminate(pid) {
                tracing::warn!("quitting pid {pid}: {e}");
            }
        }
        self.home_section = 0;
        self.go(View::Home);
        self.toast(format!("Quit {}", name.unwrap_or_else(|| "the game".into())));
    }

    /// Stops tracking the running game (its playtime so far is recorded via the `Cancelled` event).
    pub fn stop_tracking(&mut self) {
        if let Some(r) = self.running.take() {
            tracing::info!("stop tracking {} for {}", r.game_id, r.profile);
            r.handle.cancel();
        }
    }

    /// Imports Playnite's latest export into the signed-in profile's library.
    fn import_playnite(&mut self) {
        self.refresh_gamerscore();
        let Some(profile) = self.profile_id() else { return };
        self.export_mtime =
            std::fs::metadata(self.paths.root.join(playnite::EXPORT_FILE)).and_then(|m| m.modified()).ok();
        match playnite::read(&self.paths.root) {
            Ok(Some(export)) => {
                let games = playnite::imported_games(&export);
                // Win2Console's own custom games aren't managed by Playnite.
                for provider in Provider::ALL.into_iter().filter(|p| *p != Provider::Custom) {
                    let group: Vec<ImportedGame> =
                        games.iter().filter(|g| g.game.provider == provider).cloned().collect();
                    if let Err(e) = self.library.sync_library(profile, provider, &group) {
                        log_db_err::<()>(e);
                    }
                }
                self.playnite_exe = playnite::playnite_exe(Some(&export));
                self.scan_summary = format!("{} games from Playnite", games.len());
                tracing::info!("imported {} games from Playnite", games.len());
            }
            Ok(None) => {
                self.playnite_exe = playnite::playnite_exe(None);
                self.scan_summary = if self.playnite_exe.is_some() {
                    "Waiting for Playnite…".into()
                } else {
                    "Playnite isn't installed".into()
                };
            }
            Err(e) => {
                tracing::warn!("Playnite export: {e}");
                self.scan_summary = e;
            }
        }
        self.reload();
    }

    /// Imports the current export now, then asks Playnite to update its libraries in the background (it starts
    /// hidden, updates, exports and exits); the export watcher picks up the result.
    fn update_libraries(&mut self) {
        self.import_playnite();
        if self.playnite_exe.is_none() {
            self.toast("Install Playnite to import your game libraries");
            return;
        }
        match self.playnite_link("playnite://win2console/sync") {
            Ok(_) => {
                self.scanning = true;
                self.sync_started = Instant::now();
            }
            Err(e) => self.toast(format!("Couldn't start Playnite: {e}")),
        }
        self.start_export_watch();
    }

    /// Sends Playnite a `playnite://` link, starting it without its splash screen if it isn't running.
    fn playnite_link(&self, uri: &str) -> std::io::Result<Option<u32>> {
        let spec = match &self.playnite_exe {
            Some(exe) => cs_core::LaunchSpec::Exe {
                path: exe.clone(),
                args: vec!["--hidesplashscreen".into(), "--uridata".into(), format!("\"{uri}\"")],
                working_dir: exe.parent().map(Path::to_path_buf),
            },
            None => cs_core::LaunchSpec::Uri { uri: uri.into() },
        };
        cs_platform::process::launch(&spec)
    }

    fn start_export_watch(&mut self) {
        self.export_timer.start(slint::TimerMode::Repeated, Duration::from_secs(2), || {
            crate::with_app(|app| app.check_export());
        });
    }

    /// Re-imports when Playnite writes a new export (library update, install finished, ...).
    fn check_export(&mut self) {
        let mtime = std::fs::metadata(self.paths.root.join(playnite::EXPORT_FILE)).and_then(|m| m.modified()).ok();
        if mtime != self.export_mtime {
            self.import_playnite();
            if self.scanning && self.sync_started.elapsed() > Duration::from_secs(5) {
                self.scanning = false;
            }
            self.render();
        } else if self.scanning && self.sync_started.elapsed() > Duration::from_secs(180) {
            self.scanning = false;
            self.render();
        }
    }

    /// Opens Playnite, where libraries and store accounts are managed.
    fn open_playnite(&mut self) {
        match self.playnite_exe.clone() {
            Some(path) => {
                let spec = cs_core::LaunchSpec::Exe { path, args: Vec::new(), working_dir: None };
                match cs_platform::process::launch(&spec) {
                    Ok(_) => self.toast(
                        "Opening Playnite. Connect your stores there; Win2Console picks up changes automatically.",
                    ),
                    Err(e) => self.toast(format!("Couldn't open Playnite: {e}")),
                }
            }
            None => self.toast("Playnite isn't installed"),
        }
    }

    /// Plays an installed game, or starts installing it in its launcher.
    fn play(&mut self, e: &GameEntry) {
        if let Some((kind, _)) = web_kind(&e.game.id) {
            let uri = match &e.game.launch {
                cs_core::LaunchSpec::Uri { uri } if !uri.is_empty() => uri.clone(),
                _ if kind == "store" => "https://store.steampowered.com/".into(),
                _ => "https://www.xbox.com/xbox-game-pass/games".into(),
            };
            match kind {
                "store" => self.open_browse("Store", "\u{E719}", &uri),
                _ => self.open_browse("Game Pass", "\u{E7FC}", &uri),
            }
            return;
        }
        if !e.installed {
            // Not-installed games carry their Playnite install link as the launch action.
            let result = match &e.game.launch {
                cs_core::LaunchSpec::Uri { uri } if uri.starts_with("playnite://") => self.playnite_link(uri),
                spec => cs_platform::process::launch(spec),
            };
            match result {
                Ok(_) => self.toast(format!("Opening Playnite to install {}", e.game.name)),
                Err(err) => self.toast(format!("Couldn't start the install: {err}")),
            }
            return;
        }
        let game = e.game.clone();
        self.launch(&game);
    }

    fn entry_by_id(&self, id: &str) -> Option<GameEntry> {
        let p = self.profile_id()?;
        self.library.list(p, true).ok()?.into_iter().find(|e| e.game.id == id)
    }

    // ================================================================ add-ons

    // ================================================================ images

    fn image_loaded(&mut self, path: PathBuf, size: Size, pixels: Option<Pixels>) {
        self.images.insert(path, size, pixels);
        // Batch redraws while many images arrive at once.
        if !self.image_timer.running() {
            self.image_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(40), || {
                crate::with_app(|app| app.render());
            });
        }
    }

    fn save_config(&self) {
        if let Err(e) = self.config.save(&self.paths.config_file()) {
            tracing::warn!("saving config: {e}");
        }
    }

    fn toast(&mut self, message: impl Into<SharedString>) {
        let message = message.into();
        // While playing, also show it over the game.
        if self.running.is_some() && !cs_platform::window::own_window_is_foreground() {
            self.notify_over_game(&message);
        }
        self.toast_in_app(&message);
    }

    /// The toast at the bottom of Win2Console's own window.
    fn toast_in_app(&mut self, message: &str) {
        self.ui().set_toast(message.into());
        let weak = self.ui.clone();
        self.toast_timer.start(slint::TimerMode::SingleShot, TOAST_TIME, move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast(SharedString::new());
            }
        });
    }

    pub fn tick_clock(&self) {
        self.ui().set_clock(cs_platform::time::clock_text().into());
    }

    // ================================================================ render

    /// Pushes the current state to the window. Models are diffed, so this is cheap to call after
    /// every input.
    pub fn render(&mut self) {
        let ui = self.ui();
        // On the profile picker, the colours preview the focused profile.
        let shown_profile = match self.view {
            View::Profiles => self.profiles.get(self.profiles_focus).cloned(),
            _ => self.current.clone(),
        };
        let accent = shown_profile.as_ref().map_or(cs_core::model::DEFAULT_ACCENT, |p| p.accent.as_str());
        ui.set_accent(parse_color(accent));
        let background = shown_profile.as_ref().map(|p| (p.background, p.id));
        let custom_bg = match background {
            Some((Background::Custom, id)) => self.images.get(&self.store.background_path(id), Size::Backdrop),
            _ => None,
        };
        ui.set_background_mode(match background.map(|b| b.0) {
            Some(Background::Solid) => 1,
            Some(Background::Custom) if custom_bg.is_some() => 2,
            _ => 0,
        });
        ui.set_background_image(custom_bg.unwrap_or_default());

        ui.set_screen(match self.view {
            View::Splash => Screen::Splash,
            View::Profiles => Screen::Profiles,
            View::Home => Screen::Home,
            View::Library => Screen::Library,
            View::Settings => Screen::Settings,
            View::Files => Screen::Files,
            View::Search => Screen::Search,
            View::About => Screen::About,
            View::SignIn => Screen::Signin,
            View::Game => Screen::Game,
            View::MyProfile => Screen::Myprofile,
        });
        if let Some(p) = self.current.clone() {
            let item = self.profile_item(&p);
            ui.set_current_profile(item);
        }
        ui.set_controller_status(controller_text(&self.controllers).into());
        ui.set_controller_icons(controller_icons(&self.controllers).into());
        // The Guide shows the same controller battery as the home screen.
        ui.set_guide_status_icons(controller_icons(&self.controllers).into());
        ui.set_guide_status(controller_text(&self.controllers).into());
        ui.set_in_party(self.party.is_some());
        ui.set_mic_muted(self.party.as_ref().is_some_and(|p| p.self_muted));
        ui.set_deafened(self.party.as_ref().is_some_and(|p| p.self_deafened));
        ui.set_gamerscore(self.gamerscore.unwrap_or(0).to_string().into());

        match self.view {
            View::Splash => {}
            View::Profiles => self.render_profiles(&ui),
            View::Home => self.render_home(&ui),
            View::Library => self.render_library(&ui),
            View::Settings => self.render_settings(&ui),
            View::Files => self.render_files(&ui),
            View::Search => self.render_search(&ui),
            View::SignIn => self.render_signin(&ui),
            View::About => {}
            View::Game => self.render_game(&ui),
            View::MyProfile => self.render_my_profile(&ui),
        }
        self.render_modal(&ui);
        self.note_transitions();
        // Ambient music on the dashboard; quiet while a game runs.
        let music =
            !self.config.music_off && self.running.is_none() && self.view != View::Splash && self.window_focused;
        crate::sounds::set_music(music);
        if self.home_hotkey_on != self.running.is_some() {
            self.home_hotkey_on = self.running.is_some();
            if let Some(h) = &self.hotkey {
                h.set_home(self.home_hotkey_on);
            }
        }
        let hints = self.hints().into_iter().map(|(b, l)| Hint { button: b.into(), label: l.into() }).collect();
        sync(&self.models.hints, hints);
    }

    fn render_profiles(&mut self, ui: &AppWindow) {
        let items: Vec<ProfileItem> = self.profiles.clone().iter().map(|p| self.profile_item(p)).collect();
        sync(&self.models.profiles, items);
        ui.set_profiles_focus(self.profiles_focus as i32);
    }

    fn render_home(&mut self, ui: &AppWindow) {
        ui.set_home_mode(if self.home_section == 0 { 0 } else { 1 });
        let titles = self.sections.iter().map(|s| SharedString::from(s.title.as_str())).collect();
        sync(&self.models.home_sections, titles);

        let hero_tiles: Vec<Tile> = (0..self.hero.len()).map(|i| self.hero_tile(i)).collect();
        // Scroll the square row so the focused tile stays on screen (the UI slides and clips it).
        const ROW_Y: f32 = 440.0;
        let right_limit = ui.get_design_width() - 99.0;
        let offset = self
            .hero
            .get(self.hero_focus)
            .filter(|t| t.rect.y == ROW_Y)
            .map_or(self.hero_row_scroll, |t| {
                let r = t.rect.x + t.rect.w;
                if r - self.hero_row_scroll > right_limit {
                    r - right_limit
                } else if t.rect.x - self.hero_row_scroll < 536.0 {
                    t.rect.x - 536.0
                } else {
                    self.hero_row_scroll
                }
            })
            .max(0.0);
        self.hero_row_scroll = offset;
        ui.set_hero_row_scroll(offset);
        sync(&self.models.hero, hero_tiles);
        ui.set_hero_focus(self.hero_focus as i32);

        let mut backdrop = None;
        if self.home_section == 0
            && let Some(HeroItem::Game(e)) = self.home_item()
        {
            let media = &e.game.media;
            backdrop =
                media.background.clone().or(media.cover.clone()).and_then(|p| self.images.get(&p, Size::Backdrop));
        }
        if self.home_section > 0 {
            let entries = self.sections[self.home_section - 1].entries.clone();
            let tiles: Vec<Tile> = entries
                .iter()
                .enumerate()
                .map(|(i, e)| self.game_tile(e, Rect::new(99.0 + 316.0 * i as f32, 250.0, 300.0, 300.0), false))
                .collect();
            sync(&self.models.section_tiles, tiles);
            ui.set_home_section((self.home_section - 1) as i32);
            ui.set_section_focus(self.section_focus as i32);
            let right_edge = 99.0 + 316.0 * (self.section_focus + 1) as f32;
            ui.set_section_scroll((right_edge - (ui.get_design_width() - 99.0)).max(0.0));
            backdrop = entries
                .get(self.section_focus)
                .and_then(|e| e.game.media.background.clone().or(e.game.media.cover.clone()))
                .and_then(|p| self.images.get(&p, Size::Backdrop));
        }
        ui.set_has_backdrop(backdrop.is_some());
        // Keep the last image while fading out.
        if let Some(b) = backdrop {
            ui.set_backdrop(b);
        }
    }

    fn render_library(&mut self, ui: &AppWindow) {
        let entries = self.lib_entries();
        let mut y = 180.0;
        let categories = entries
            .iter()
            .enumerate()
            .map(|(i, &entry)| {
                let divider = entry == LibEntry::Hidden;
                if divider {
                    y += 40.0;
                }
                let label = match entry {
                    LibEntry::All => "All games".to_string(),
                    LibEntry::Installed => "Installed".into(),
                    LibEntry::Provider(p) => p.display_name().to_string(),
                    LibEntry::Hidden => "Hidden".into(),
                    LibEntry::ManageAddons => "Manage libraries".into(),
                    LibEntry::Update => {
                        if self.scanning {
                            "Updating…".into()
                        } else {
                            "Update game library".into()
                        }
                    }
                };
                let count = match entry {
                    LibEntry::ManageAddons | LibEntry::Update => String::new(),
                    _ => self.lib_counts.get(i).copied().unwrap_or(0).to_string(),
                };
                let item = SidebarItem { label: label.into(), count: count.into(), y, divider_above: divider };
                y += 72.0;
                item
            })
            .collect();
        sync(&self.models.lib_categories, categories);
        ui.set_lib_category(self.lib_category as i32);
        ui.set_lib_sidebar_focused(self.lib_zone == LibZone::Sidebar);
        ui.set_lib_title(
            match self.lib_entry() {
                LibEntry::Provider(p) => p.display_name(),
                LibEntry::Hidden => "Hidden games",
                LibEntry::Installed => "Installed games",
                _ => "Games",
            }
            .into(),
        );
        ui.set_lib_sort(self.lib_sort.label().into());
        ui.set_lib_control_focus(match self.lib_zone {
            LibZone::Controls(c) => c as i32,
            _ => -1,
        });

        let (origin, cols) = self.lib_layout();
        let games = self.lib_games.clone();
        let focus_row = if self.lib_zone == LibZone::Grid { self.lib_focus / cols } else { 0 };
        let tiles: Vec<Tile> = games
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let rect =
                    Rect::new(origin + (i % cols) as f32 * 272.0, 400.0 + (i / cols) as f32 * 272.0, 256.0, 256.0);
                // Load art for the rows on screen and a couple either side; the rest use what's cached.
                self.lazy_images = (i / cols) + 2 < focus_row || i / cols > focus_row + 5;
                let tile = self.game_tile(e, rect, false);
                self.lazy_images = false;
                tile
            })
            .collect();
        sync(&self.models.games, tiles);
        ui.set_lib_focus(if self.lib_zone == LibZone::Grid { self.lib_focus as i32 } else { -1 });
        let scroll = match self.lib_zone {
            LibZone::Grid => (400.0 + (self.lib_focus / cols) as f32 * 272.0 - 420.0).max(0.0),
            _ => 0.0,
        };
        ui.set_lib_scroll(scroll);
        ui.set_lib_empty(
            if self.scanning { "Looking for games…" } else { "Nothing here yet. Choose Add a game to add one." }.into(),
        );
    }

    fn render_settings(&mut self, ui: &AppWindow) {
        ui.set_settings_glyph("\u{E713}".into());
        sync(&self.models.settings_categories, SETTINGS_CATEGORIES.iter().map(|&c| c.into()).collect());
        ui.set_settings_category(self.settings_category as i32);
        ui.set_settings_in_content(self.settings_in_content);
        ui.set_settings_title(SETTINGS_CATEGORIES[self.settings_category].into());
        let tiles = settings_for(self.settings_category)
            .iter()
            .zip(Self::setting_rects(self.settings_category))
            .map(|(&s, r)| self.setting_tile(s, r))
            .collect();
        sync(&self.models.settings_tiles, tiles);
        ui.set_settings_focus(self.settings_focus as i32);
    }

    fn render_files(&mut self, ui: &AppWindow) {
        let Some((fb, _, _)) = &self.files else { return };
        ui.set_list_title(fb.title.clone().into());
        ui.set_list_subtitle(fb.location().into());
        let items = fb
            .entries
            .iter()
            .map(|e| {
                let glyph = match e.kind {
                    EntryKind::Up => "\u{E74A}",
                    EntryKind::Place | EntryKind::Dir => "\u{ED25}",
                    EntryKind::Drive => "\u{EDA2}",
                    EntryKind::File => "\u{E7C3}",
                };
                ListItem { glyph: glyph.into(), label: e.name.clone().into(), value: SharedString::new() }
            })
            .collect();
        sync(&self.models.list_items, items);
        ui.set_list_focus(fb.focus as i32);
    }

    fn render_search(&mut self, ui: &AppWindow) {
        let Some(s) = &self.search else { return };
        let (before, after) = s.osk.text_around_cursor();
        ui.set_kb_before(before.into());
        ui.set_kb_after(after.into());
        ui.set_kb_placeholder(s.osk.placeholder.clone().into());
        let keys = key_views(&s.osk);
        let kb_focus = s.osk.focus;
        let focus = s.focus;
        let results = s.results.clone();
        sync(&self.models.kb_keys, keys);
        ui.set_kb_focus(kb_focus as i32);
        let tiles = results
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let mut t = self.game_tile(e, Rect::new(105.0 + 216.0 * i as f32, 250.0, 200.0, 200.0), false);
                t.caption = true;
                t
            })
            .collect();
        sync(&self.models.search_results, tiles);
        ui.set_search_focus(focus.map_or(-1, |f| f as i32));
    }

    /// The Guide panel's tabs and list (also drawn behind a friend's card).
    fn render_guide(&mut self, ui: &AppWindow, tab: usize, focus: usize) {
        sync(
            &self.models.guide_tabs,
            GUIDE_TABS.iter().map(|g| GuideTab { glyph: (*g).into(), avatar: g.is_empty() }).collect(),
        );
        ui.set_guide_tab(tab as i32);
        if tab == TAB_ACHIEVEMENTS && self.ach_guide.as_ref().is_none_or(|a| a.at.elapsed() > Duration::from_secs(20)) {
            self.load_ach_guide();
        }
        if tab == TAB_CAPTURE && self.captures_at.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.refresh_captures();
        }
        let entries = self.guide_entries(tab);
        let items = entries
            .iter()
            .map(|e| {
                let image = e.image.as_ref().and_then(|p| self.images.get(p, Size::Tile));
                GuideItem {
                    y: e.y,
                    glyph: e.glyph.into(),
                    label: e.label.clone().into(),
                    subtitle: e.subtitle.clone().into(),
                    has_image: image.is_some(),
                    image: image.unwrap_or_default(),
                    solid: e.solid,
                    thumb: e.thumb,
                    divider_above: e.divider_above,
                    round: e.round,
                    presence: e.presence,
                    others: Rc::new(VecModel::from(
                        e.others.clone().iter().map(|p| self.profile_item(p)).collect::<Vec<_>>(),
                    ))
                    .into(),
                    more: e.more.clone().into(),
                    profile_row: e.profile_row,
                    score: e.score,
                }
            })
            .collect();
        sync(&self.models.guide_items, items);
        // Keep the focused row inside the visible list area (rows are 96 tall; area starts at y 128).
        let panel_h = 1014.0_f32.min(ui.get_design_height() - 66.0);
        let (top, bottom) = (128.0, panel_h - 130.0);
        match entries.get(focus) {
            Some(e) => {
                if e.y - self.guide_scroll < top + 8.0 {
                    self.guide_scroll = (e.y - top - 8.0).max(0.0);
                } else if e.y + 96.0 - self.guide_scroll > bottom - 8.0 {
                    self.guide_scroll = e.y + 96.0 - (bottom - 8.0);
                }
            }
            None => {}
        }
        if focus == 0 {
            self.guide_scroll = 0.0;
        }
        ui.set_guide_scroll(self.guide_scroll);
        sync(&self.models.guide_buttons, GUIDE_BUTTONS.iter().map(|(g, _)| (*g).into()).collect());
        ui.set_guide_focus(focus as i32);
        self.render_guide_chat(ui);
    }

    fn render_modal(&mut self, ui: &AppWindow) {
        if !matches!(self.modal, Some(ModalState::Viewer { .. })) {
            self.viewer_video = None;
        }
        match &mut self.ingame {
            Some(g) if !g.shown => {
                ui.set_ingame_backdrop(g.backdrop.clone().unwrap_or_default());
                ui.set_ingame(true);
                g.shown = true;
            }
            Some(_) => {}
            None if ui.get_ingame() => {
                ui.set_ingame(false);
                ui.set_ingame_backdrop(slint::Image::default()); // free the capture
            }
            None => {}
        }
        match &self.modal {
            None => ui.set_modal(Modal::None),
            Some(ModalState::Power { focus }) => {
                ui.set_modal(Modal::Power);
                ui.set_menu_focus(*focus as i32);
                let items =
                    POWER_ITEMS.iter().map(|(g, l, _)| MenuItem { glyph: (*g).into(), label: (*l).into() }).collect();
                sync(&self.models.menu_items, items);
            }
            Some(ModalState::Viewer { index }) => {
                let index = *index;
                ui.set_modal(Modal::Viewer);
                let path = self.captures.get(index).cloned();
                match &path {
                    Some(p) if is_video(p) && self.viewer_video.as_ref().is_none_or(|(i, _)| *i != index) => {
                        self.viewer_video = None;
                        let cache = self.paths.root.join("cache");
                        match crate::video::StartupVideo::open(ui.window(), &cache, p, || {}) {
                            Ok(v) => self.viewer_video = Some((index, v)),
                            Err(e) => tracing::warn!("playing {}: {e}", p.display()),
                        }
                    }
                    Some(p) if !is_video(p) => self.viewer_video = None,
                    _ => {}
                }
                let img = path.as_ref().and_then(|p| self.images.get(p, Size::Backdrop));
                ui.set_viewer_image(img.unwrap_or_default());
                let title = path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned());
                ui.set_viewer_title(title.unwrap_or_default().into());
            }
            Some(ModalState::Audio { focus, sources, .. }) => {
                ui.set_modal(Modal::Audio);
                let rows = sources
                    .iter()
                    .map(|s| {
                        let (glyph, label) = match s.kind {
                            AudioKind::Master => ("\u{E767}", "Volume"),
                            AudioKind::Game => ("\u{E7FC}", "Game"),
                            AudioKind::Party => ("\u{E716}", "Party chat"),
                            AudioKind::MicVolume => ("\u{E720}", "Microphone volume"),
                            AudioKind::NoiseSuppression => ("\u{E9D9}", "Noise suppression"),
                            AudioKind::Microphone => ("\u{E720}", "Microphone"),
                            AudioKind::Speakers => ("\u{E7F5}", "Speakers"),
                        };
                        AudioRow {
                            glyph: glyph.into(),
                            label: label.into(),
                            level: s.level,
                            muted: s.muted,
                            value: s.value.clone().unwrap_or_default().into(),
                            slider: s.value.is_none(),
                        }
                    })
                    .collect();
                ui.set_audio_focus(*focus as i32);
                sync(&self.models.audio_rows, rows);
            }
            Some(ModalState::Guide { tab, focus }) => {
                let (tab, focus) = (*tab, *focus);
                ui.set_modal(Modal::Guide);
                self.render_guide(ui, tab, focus);
            }
            Some(ModalState::Chats { focus, tab, guide_focus }) => {
                let (focus, tab, guide_focus) = (*focus, *tab, *guide_focus);
                ui.set_modal(Modal::Chats);
                self.render_guide(ui, tab, guide_focus);
                let rows: Vec<GuideItem> = self
                    .recent_chats(7)
                    .into_iter()
                    .map(|peer| {
                        let f = self.person(peer);
                        let last = self
                            .chats
                            .get(&peer)
                            .and_then(|m| m.last())
                            .map(|m| if m.from_me { format!("You: {}", m.text) } else { m.text.clone() });
                        let image =
                            f.as_ref().and_then(|f| f.avatar.clone()).and_then(|p| self.images.get(&p, Size::Tile));
                        GuideItem {
                            label: f.as_ref().map_or("Discord user".into(), |f| f.name.clone()).into(),
                            subtitle: last.unwrap_or_default().into(),
                            has_image: image.is_some(),
                            image: image.unwrap_or_default(),
                            presence: f.as_ref().map_or(-1, |f| presence_code(f.presence)),
                            ..Default::default()
                        }
                    })
                    .collect();
                ui.set_chat_rows(Rc::new(VecModel::from(rows)).into());
                ui.set_menu_focus(focus as i32);
            }
            Some(ModalState::Friend { id, focus, tab, guide_focus }) => {
                let (id, focus, tab, guide_focus) = (*id, *focus, *tab, *guide_focus);
                ui.set_modal(Modal::Friend);
                self.render_guide(ui, tab, guide_focus);
                let f = self.person(id);
                let picture = f.as_ref().and_then(|f| f.avatar.clone()).and_then(|p| self.images.get(&p, Size::Tile));
                ui.set_friend_name(f.as_ref().map(|f| f.name.clone()).unwrap_or_default().into());
                ui.set_friend_status(f.as_ref().map(|f| f.presence.label().to_string()).unwrap_or_default().into());
                ui.set_friend_playing(f.as_ref().and_then(|f| f.activity.clone()).unwrap_or_default().into());
                ui.set_friend_presence(f.as_ref().map_or(-1, |f| presence_code(f.presence)));
                ui.set_friend_has_picture(picture.is_some());
                ui.set_friend_picture(picture.unwrap_or_default());
                let items = self
                    .friend_actions(id)
                    .into_iter()
                    .map(|(g, l, _)| MenuItem { glyph: g.into(), label: l.into() })
                    .collect();
                sync(&self.models.menu_items, items);
                ui.set_menu_focus(focus as i32);
            }
            Some(ModalState::Menu { title, message, entries, focus }) => {
                ui.set_modal(Modal::Menu);
                ui.set_menu_title(title.into());
                ui.set_menu_message(message.into());
                let items =
                    entries.iter().map(|e| MenuItem { glyph: e.glyph.into(), label: e.label.clone().into() }).collect();
                let focus = *focus;
                sync(&self.models.menu_items, items);
                ui.set_menu_focus(focus as i32);
            }
            Some(ModalState::Picker { kind, focus }) => {
                let (kind, focus) = (*kind, *focus);
                ui.set_modal(Modal::Picker);
                ui.set_picker_focus(focus as i32);
                ui.set_picker_columns(picker_shape(kind, self.avatar_pack.len()).1 as i32);
                ui.set_picker_round(matches!(kind, PickerKind::Avatar { .. }));
                let p = self.current.clone();
                let accent = parse_color(p.as_ref().map_or(cs_core::model::DEFAULT_ACCENT, |p| &p.accent));
                let grey = parse_color("#3A3A3A");
                let items: Vec<PickerItem> = match kind {
                    PickerKind::Accent { .. } => {
                        ui.set_picker_title("My color".into());
                        let mut v: Vec<PickerItem> = ACCENTS
                            .iter()
                            .map(|(hex, name)| {
                                let current = p.as_ref().is_some_and(|p| p.accent == *hex);
                                picker_item("", parse_color(hex), None, name, current)
                            })
                            .collect();
                        v.push(picker_item("\u{E765}", grey, None, "Custom", false));
                        v
                    }
                    PickerKind::Avatar { .. } => {
                        ui.set_picker_title("Profile picture".into());
                        let mut v: Vec<PickerItem> = (0..BUILTIN_AVATARS)
                            .map(|i| picker_item(avatar_glyph(i), accent, None, "", false))
                            .collect();
                        let custom = p
                            .as_ref()
                            .filter(|p| p.avatar == Avatar::Custom)
                            .and_then(|p| self.images.get(&self.store.avatar_path(p.id), Size::Tile));
                        v.push(picker_item("\u{EB9F}", grey, custom, "From file", false));
                        let pack = self.avatar_pack.clone();
                        for path in &pack {
                            let img = self.images.get(path, Size::Tile);
                            v.push(picker_item("\u{E77B}", grey, img, "", false));
                        }
                        v
                    }
                    PickerKind::Captures { .. } => {
                        ui.set_picker_title("Captures".into());
                        let captures = self.captures.clone();
                        captures
                            .iter()
                            .map(|c| {
                                let img = self.images.get(c, Size::Tile);
                                picker_item("\u{E722}", grey, img, "", false)
                            })
                            .collect()
                    }
                    PickerKind::Background => {
                        ui.set_picker_title("My background".into());
                        let bg = p.as_ref().map(|p| p.background).unwrap_or_default();
                        let custom = p
                            .as_ref()
                            .filter(|p| p.background == Background::Custom)
                            .and_then(|p| self.images.get(&self.store.background_path(p.id), Size::Backdrop));
                        vec![
                            picker_item("\u{E790}", accent.darker(0.4), None, "Default", bg == Background::Default),
                            picker_item("", accent.darker(0.2), None, "Solid color", bg == Background::Solid),
                            picker_item("\u{EB9F}", grey, custom, "Custom image", bg == Background::Custom),
                        ]
                    }
                };
                sync(&self.models.picker_items, items);
            }
            Some(ModalState::Keyboard { osk, .. }) => {
                ui.set_modal(Modal::Keyboard);
                ui.set_kb_title(osk.title.clone().into());
                let (before, after) = osk.text_around_cursor();
                ui.set_kb_before(before.into());
                ui.set_kb_after(after.into());
                ui.set_kb_placeholder(osk.placeholder.clone().into());
                let keys = key_views(osk);
                let focus = osk.focus;
                sync(&self.models.kb_keys, keys);
                ui.set_kb_focus(focus as i32);
            }
        }
    }

    /// Advances the GIFs in the open chat; stops once none are showing.
    fn tick_gifs(&mut self) {
        let model = &self.models.chat_lines;
        let mut any = false;
        for i in 0..model.row_count() {
            let Some(mut line) = model.row_data(i) else { continue };
            let gifs = self.gifs.get_mut();
            if line.url.is_empty() || !gifs.animated(&line.url) {
                continue;
            }
            any = true;
            if let Some(image) = gifs.frame(&line.url) {
                line.image = image;
                model.set_row_data(i, line);
            }
        }
        if !any || !self.ui().get_chat_mode() {
            self.gif_timer.stop();
        }
    }

    pub fn gif_loaded(&mut self, url: String, frames: Option<Vec<(image::RgbaImage, u32)>>) {
        self.gifs.get_mut().insert(url, frames);
        self.render();
    }

    pub fn set_window_focused(&mut self, focused: bool) {
        // Switching back to Win2Console by hand ends desktop mode.
        if focused && self.desktop && self.ingame.is_none() {
            self.leave_desktop();
        }
        if self.window_focused != focused {
            self.window_focused = focused;
            self.render();
        }
    }

    fn render_guide_chat(&self, ui: &AppWindow) {
        let Some(chat) = &self.guide_chat else {
            ui.set_chat_mode(false);
            ui.set_composing(false);
            return;
        };
        ui.set_chat_mode(true);
        let friend = self.person(chat.peer);
        let party_chat = self.party.as_ref().is_some_and(|p| p.chat == chat.peer);
        let title = friend
            .as_ref()
            .map_or_else(|| if party_chat { "Party chat".into() } else { "Chat".into() }, |f| f.name.clone());
        ui.set_chat_title(title.into());
        ui.set_chat_subtitle(friend.as_ref().map_or(String::new(), |f| f.status_line()).into());
        let lines = self
            .chats
            .get(&chat.peer)
            .map(|msgs| {
                let end = msgs.len().saturating_sub(chat.scroll);
                let start = end.saturating_sub(12);
                msgs[start..end]
                    .iter()
                    .map(|m| {
                        let mut line = ChatLine {
                            text: m.text.clone().into(),
                            mine: m.from_me,
                            time: time_ago(m.at).into(),
                            ..Default::default()
                        };
                        if let Some((url, rest)) = crate::gifs::find_link(&m.text)
                            && let Some(image) = self.gifs.borrow_mut().frame(&url)
                        {
                            line.text = rest.into();
                            line.image = image;
                            line.has_image = true;
                            line.url = url.into();
                        }
                        line
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let animated = lines.iter().any(|l| self.gifs.borrow().animated(&l.url));
        sync(&self.models.chat_lines, lines);
        if animated && !self.gif_timer.running() {
            self.gif_timer.start(slint::TimerMode::Repeated, Duration::from_millis(40), || {
                crate::with_app(|app| app.tick_gifs());
            });
        }
        match &chat.compose {
            Some(osk) => {
                ui.set_composing(true);
                ui.set_compose_text(osk.text().into());
                sync(&self.models.kb_keys, key_views(osk));
                ui.set_kb_focus(osk.focus as i32);
            }
            None => {
                ui.set_composing(false);
                ui.set_compose_text(SharedString::new());
            }
        }
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        match &self.modal {
            Some(ModalState::Guide { .. }) => {
                return match &self.guide_chat {
                    Some(GuideChat { compose: Some(_), .. }) => vec![("≡", "Send"), ("B", "Cancel")],
                    Some(c) if c.scroll > 0 => vec![("A", "Write a message"), ("X", "Latest"), ("B", "Back")],
                    Some(_) => vec![("A", "Write a message"), ("B", "Back")],
                    None => vec![("LB", "Prev tab"), ("RB", "Next tab"), ("B", "Close")],
                };
            }
            Some(ModalState::Menu { .. } | ModalState::Picker { .. }) => return vec![("A", "Select"), ("B", "Back")],
            Some(ModalState::Audio { .. }) => return vec![("A", "Mute"), ("B", "Back")],
            Some(ModalState::Viewer { .. }) => return vec![("X", "Delete"), ("B", "Back")],
            Some(ModalState::Power { .. }) => return vec![("A", "Select"), ("B", "Close")],
            Some(ModalState::Friend { .. } | ModalState::Chats { .. }) => return vec![("A", "Select"), ("B", "Back")],
            Some(ModalState::Keyboard { .. }) => return Vec::new(),
            None => {}
        }
        match self.view {
            View::Splash | View::Profiles => Vec::new(),
            View::Game | View::MyProfile => vec![("A", "Select"), ("B", "Back")],
            View::Home => match self.home_item() {
                Some(HeroItem::Game(_)) => vec![("≡", "More options"), ("Y", "Search")],
                _ => vec![("View", "Customize"), ("Y", "Search")],
            },
            View::Library => match self.lib_zone {
                LibZone::Grid => vec![("≡", "More options"), ("Y", "Sort")],
                _ => vec![("View", "Add a game")],
            },
            View::Files => vec![("Y", "This PC"), ("B", "Up")],
            View::Search => match self.search.as_ref().and_then(|s| s.focus) {
                Some(_) => vec![("≡", "More options")],
                None => Vec::new(),
            },
            View::Settings | View::About => Vec::new(),
            View::SignIn => match self.signin.as_ref().map(|s| s.zone) {
                Some(SignInZone::Keyboard) => vec![("≡", "Done"), ("B", "Back")],
                _ => vec![("A", "Select"), ("≡", "Submit"), ("B", "Cancel")],
            },
        }
    }

    fn hero_tile(&mut self, i: usize) -> Tile {
        let (rect, item, wide, caption) = {
            let t = &self.hero[i];
            (t.rect, t.item.clone(), t.wide, t.caption)
        };
        match item {
            HeroItem::Game(e) => {
                let mut tile = self.game_tile(&e, rect, wide);
                tile.caption = caption;
                tile
            }
            HeroItem::Shortcut(s) => {
                Tile { title: s.label().into(), glyph: s.glyph().into(), style: STYLE_SOLID, caption, ..tile_at(rect) }
            }
            HeroItem::Library => {
                // Just the icon and label (no game art).
                Tile {
                    title: "My games & apps".into(),
                    glyph: "\u{E8F1}".into(),
                    style: STYLE_DARK,
                    caption: true,
                    ..tile_at(rect)
                }
            }
        }
    }

    /// A game tile. `wide` tiles prefer landscape background art and show a subtitle.
    fn game_tile(&mut self, e: &GameEntry, rect: Rect, wide: bool) -> Tile {
        let media = &e.game.media;
        let image = if wide {
            media
                .background
                .as_ref()
                .and_then(|p| self.images.get(p, Size::Wide))
                .or_else(|| media.cover.as_ref().and_then(|p| self.images.get(p, Size::Tile)))
        } else if self.lazy_images {
            media.cover.as_ref().and_then(|p| self.images.peek(p, Size::Tile))
        } else {
            media.cover.as_ref().and_then(|p| self.images.get(p, Size::Tile))
        };
        Tile {
            title: e.game.name.clone().into(),
            subtitle: if wide { game_subtitle(e).into() } else { SharedString::new() },
            glyph: "\u{E7FC}".into(),
            has_image: image.is_some(),
            image: image.unwrap_or_default(),
            style: STYLE_ART,
            pinned: e.stats.pinned,
            badge: if e.installed { SharedString::new() } else { "\u{E896}".into() },
            ..tile_at(rect)
        }
    }

    fn profile_item(&mut self, p: &Profile) -> ProfileItem {
        let custom = match p.avatar {
            Avatar::Custom => self.images.get(&self.store.avatar_path(p.id), Size::Tile),
            Avatar::Builtin { .. } => None,
        };
        ProfileItem {
            name: p.name.clone().into(),
            accent: parse_color(&p.accent),
            avatar_index: match p.avatar {
                Avatar::Builtin { index } => index as i32,
                Avatar::Custom => 0,
            },
            has_custom_avatar: custom.is_some(),
            custom_avatar: custom.unwrap_or_default(),
        }
    }
}

/// OAuth scopes requested when linking Discord (the SDK's own list when it's available).
fn discord_scopes() -> String {
    #[cfg(discord_sdk)]
    {
        crate::discord::communication_scopes()
    }
    #[cfg(not(discord_sdk))]
    {
        "openid sdk.social_layer".to_string()
    }
}

fn presence_code(p: Presence) -> i32 {
    match p {
        Presence::Online => 0,
        Presence::Idle => 1,
        Presence::DoNotDisturb => 2,
        Presence::Offline => 3,
    }
}

/// "now", "5m", "3h", "2d".
/// The native handle of one of our secondary windows (notifications, pop-ups), once it exists.
fn overlay_hwnd(window: &slint::Window) -> Option<isize> {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
            _ => None,
        })
        .flatten()
}

fn time_ago(at: i64) -> String {
    let secs = (cs_core::now_unix() - at).max(0);
    match secs {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

fn tile_at(rect: Rect) -> Tile {
    Tile {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        title: SharedString::new(),
        subtitle: SharedString::new(),
        glyph: SharedString::new(),
        image: Image::default(),
        has_image: false,
        style: STYLE_ART,
        caption: false,
        strip: false,
        pinned: false,
        badge: SharedString::new(),
    }
}

fn key_views(osk: &Osk) -> Vec<KeyView> {
    osk.keys()
        .into_iter()
        .map(|k| KeyView {
            x: k.rect.x,
            y: k.rect.y,
            w: k.rect.w,
            h: k.rect.h,
            label: k.label.into(),
            hint: k.hint.into(),
            active: k.active,
        })
        .collect()
}

fn game_subtitle(e: &GameEntry) -> String {
    if e.stats.playtime_secs >= 60 {
        format_playtime(e.stats.playtime_secs)
    } else {
        e.game.provider.display_name().to_string()
    }
}

fn entry(glyph: &'static str, label: &str, action: MenuAction) -> MenuEntry {
    MenuEntry { glyph, label: label.into(), action }
}

fn picker_item(glyph: &str, swatch: Color, image: Option<Image>, label: &str, current: bool) -> PickerItem {
    PickerItem {
        glyph: glyph.into(),
        swatch,
        has_image: image.is_some(),
        image: image.unwrap_or_default(),
        label: label.into(),
        current,
    }
}

/// Item count and columns; `pack` is the number of pictures in the avatars folder.
fn picker_shape(kind: PickerKind, pack: usize) -> (usize, usize) {
    match kind {
        PickerKind::Accent { .. } => (ACCENTS.len() + 1, 4),
        PickerKind::Avatar { .. } if pack > 0 => (BUILTIN_AVATARS + 1 + pack, 6),
        PickerKind::Avatar { .. } => (BUILTIN_AVATARS + 1, 3),
        PickerKind::Background => (3, 3),
        PickerKind::Captures { count } => (count, 6),
    }
}

/// Must match `Icons.avatars` in ui/theme.slint.
fn avatar_glyph(i: usize) -> &'static str {
    ["\u{E7FC}", "\u{E735}", "\u{EB52}", "\u{E945}", "\u{E909}", "\u{E706}", "\u{E708}", "\u{E8D6}"]
        [i % BUILTIN_AVATARS]
}

fn controller_text(controllers: &[ControllerInfo]) -> String {
    if controllers.is_empty() { "No controller".into() } else { String::new() }
}

/// A battery glyph per wireless controller (Segoe MDL2 Battery0–Battery10); wired ones show nothing.
fn controller_icons(controllers: &[ControllerInfo]) -> String {
    let mut s = String::new();
    for c in controllers {
        if !s.is_empty() {
            s.push_str("  ");
        }
        if let Some(p) = c.battery_percent {
            let level = ((p as u32 + 5) / 10).min(10);
            s.push(if level == 10 { '\u{E83F}' } else { char::from_u32(0xE850 + level).unwrap_or('\u{E850}') });
        }
    }
    s
}

pub fn format_playtime(secs: i64) -> String {
    match secs {
        s if s < 60 => "Not played yet".into(),
        s if s < 3600 => format!("{} min played", s / 60),
        s => format!("{:.1} h played", s as f64 / 3600.0),
    }
}

fn parse_color(hex: &str) -> Color {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x107C10);
    Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn log_db_err<T: Default>(e: cs_core::library::DbError) -> T {
    tracing::error!("database: {e}");
    T::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playtime_text() {
        assert_eq!(format_playtime(0), "Not played yet");
        assert_eq!(format_playtime(125), "2 min played");
        assert_eq!(format_playtime(5400), "1.5 h played");
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#107C10"), Color::from_rgb_u8(0x10, 0x7C, 0x10));
    }

    #[test]
    fn accent_palette_is_valid() {
        for (hex, _) in ACCENTS {
            assert_eq!(cs_core::model::parse_accent(hex).as_deref(), Some(hex));
        }
    }

    #[test]
    fn settings_tiles_are_navigable() {
        for c in 0..SETTINGS_CATEGORIES.len() {
            let rects = App::setting_rects(c);
            assert_eq!(rects.len(), settings_for(c).len());
            if rects.len() > 1 {
                assert_eq!(spatial_move(&rects, 0, NavAction::Right), Some(1));
            }
        }
    }
}

/// How the achievements add-on identifies a game: `playnite:<id>` games by Playnite id, store games by app id.
fn game_key(game: &Game) -> cs_library::achievements::GameKey<'_> {
    let (kind, rest) = game.id.split_once(':').unwrap_or(("", &game.id));
    cs_library::achievements::GameKey {
        playnite_id: (kind == "playnite").then_some(rest),
        provider_id: (kind != "playnite").then(|| rest.parse().ok()).flatten(),
        name: &game.name,
    }
}

/// Home row items that open a web page (`store:…` / `gamepass:…`) rather than a game.
fn web_kind(id: &str) -> Option<(&str, &str)> {
    id.split_once(':').filter(|(k, _)| *k == "store" || *k == "gamepass")
}

fn web_entry(id: &str, name: &str, image: Option<String>, page: &str) -> GameEntry {
    GameEntry {
        game: Game {
            id: id.into(),
            provider: Provider::Other,
            name: name.into(),
            install_dir: None,
            launch: cs_core::LaunchSpec::Uri { uri: page.into() },
            media: cs_core::GameMedia { cover: image.map(PathBuf::from), icon: None, background: None },
        },
        installed: true,
        stats: Default::default(),
    }
}
