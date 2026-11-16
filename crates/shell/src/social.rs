//! Friends, presence, chat and parties. The UI talks to a `SocialBackend`; the Discord Social SDK
//! backend implements it. A demo backend (env `WIN2CONSOLE_DEMO_SOCIAL=1`) exists only to develop the
//! UI without a Discord application.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Presence {
    Online,
    Idle,
    DoNotDisturb,
    Offline,
}

impl Presence {
    pub fn label(self) -> &'static str {
        match self {
            Presence::Online => "Online",
            Presence::Idle => "Away",
            Presence::DoNotDisturb => "Do not disturb",
            Presence::Offline => "Offline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Friend {
    pub id: u64,
    pub name: String,
    /// Downloaded avatar image, if any.
    pub avatar: Option<PathBuf>,
    pub presence: Presence,
    /// Rich presence, e.g. "Playing Forza Horizon 5".
    pub activity: Option<String>,
}

impl Friend {
    pub fn status_line(&self) -> String {
        self.activity.clone().unwrap_or_else(|| self.presence.label().to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub id: u64,
    pub from_me: bool,
    pub text: String,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyMember {
    pub id: u64,
    pub name: String,
    pub avatar: Option<PathBuf>,
    pub speaking: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Party {
    /// Text chat for the party (a conversation id like a friend's).
    pub chat: u64,
    pub members: Vec<PartyMember>,
    pub self_muted: bool,
    pub self_deafened: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    NotLinked,
    Connecting,
    Connected { user: String },
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum Event {
    Status(Status),
    Friends(Vec<Friend>),
    Message {
        peer: u64,
        message: Message,
    },
    Party(Option<Party>),
    PartyInvite {
        from: u64,
        name: String,
    },
    /// Incoming friend requests.
    Requests(Vec<Friend>),
    /// Something to tell the user (e.g. "Friend request sent").
    Notice(String),
    Audio(AudioInfo),
}

/// Party chat audio settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioInfo {
    /// (id, name)
    pub inputs: Vec<(String, String)>,
    pub outputs: Vec<(String, String)>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub noise_suppression: bool,
    /// 0.0–1.0
    pub mic_volume: f32,
}

impl AudioInfo {
    pub fn device_name(list: &[(String, String)], id: &Option<String>) -> String {
        id.as_ref()
            .and_then(|id| list.iter().find(|(d, _)| d == id))
            .map_or_else(|| "Default".to_string(), |(_, n)| n.clone())
    }
}

/// What the UI asks of a social backend. All calls are made on the UI thread; events come back through
/// the callback given at construction.
pub trait SocialBackend {
    fn send_message(&mut self, peer: u64, text: &str);
    /// Starts a party (if needed) and invites `peer` to it.
    fn invite_to_party(&mut self, peer: u64);
    fn accept_party_invite(&mut self, from: u64);
    fn leave_party(&mut self);
    fn set_muted(&mut self, muted: bool);
    fn set_deafened(&mut self, deafened: bool);
    fn send_friend_request(&mut self, _username: &str) {}
    fn answer_friend_request(&mut self, _from: u64, _accept: bool) {}
    /// Switches to the next microphone (`input`) or speaker.
    fn next_audio_device(&mut self, _input: bool) {}
    fn set_noise_suppression(&mut self, _on: bool) {}
    fn set_mic_volume(&mut self, _volume: f32) {}
    /// Fetches recent messages with `peer` (delivered as `Event::Message`s).
    fn load_history(&mut self, _peer: u64) {}
    /// Shares what you're playing (`None` = back on the dashboard) with friends.
    fn set_playing(&mut self, _game: Option<&str>) {}
    /// Party chat (speaker) volume, 0.0–1.0, if the backend controls it.
    fn output_volume(&mut self) -> Option<f32> {
        None
    }
    fn set_output_volume(&mut self, _volume: f32) {}
    /// Called regularly on the UI thread (the Discord SDK delivers callbacks here).
    fn tick(&mut self);
}

/// Development-only backend with fixed friends, so the social UI can be built and screenshotted.
pub struct DemoBackend {
    emit: Box<dyn Fn(Event)>,
    party: Option<Party>,
    next_id: u64,
}

impl DemoBackend {
    pub fn new(emit: impl Fn(Event) + 'static) -> Self {
        let backend = Self { emit: Box::new(emit), party: None, next_id: 100 };
        (backend.emit)(Event::Status(Status::Connected { user: "demo".into() }));
        let friend = |id, name: &str, presence, activity: Option<&str>| Friend {
            id,
            name: name.into(),
            avatar: None,
            presence,
            activity: activity.map(Into::into),
        };
        (backend.emit)(Event::Friends(vec![
            friend(1, "Nova", Presence::Online, Some("Playing Forza Horizon 5")),
            friend(2, "Pixelwolf", Presence::Online, Some("In Win2Console")),
            friend(3, "Kestrel", Presence::Idle, None),
            friend(4, "Moss", Presence::DoNotDisturb, Some("Playing Halo Infinite")),
            friend(5, "Juniper", Presence::Offline, None),
        ]));
        let now = cs_core::now_unix();
        for (peer, from_me, text, ago) in [
            (1, false, "you on tonight?", 600),
            (1, true, "yeah after dinner", 540),
            (1, false, "cool, I'll start a party", 500),
            (2, false, "gg", 7200),
        ] {
            (backend.emit)(Event::Message {
                peer,
                message: Message { id: ago as u64, from_me, text: text.into(), at: now - ago },
            });
        }
        backend
    }

    fn emit_party(&self) {
        (self.emit)(Event::Party(self.party.clone()));
    }
}

impl SocialBackend for DemoBackend {
    fn send_message(&mut self, peer: u64, text: &str) {
        self.next_id += 1;
        let message = Message { id: self.next_id, from_me: true, text: text.into(), at: cs_core::now_unix() };
        (self.emit)(Event::Message { peer, message });
    }

    fn invite_to_party(&mut self, peer: u64) {
        let party = self.party.get_or_insert_with(Party::default);
        if !party.members.iter().any(|m| m.id == peer) {
            let names = ["", "Nova", "Pixelwolf", "Kestrel", "Moss", "Juniper"];
            let name = names.get(peer as usize).copied().unwrap_or("Friend").to_string();
            party.members.push(PartyMember { id: peer, name, avatar: None, speaking: peer == 1, muted: false });
        }
        self.emit_party();
    }

    fn accept_party_invite(&mut self, from: u64) {
        self.invite_to_party(from);
    }

    fn leave_party(&mut self) {
        self.party = None;
        self.emit_party();
    }

    fn set_muted(&mut self, muted: bool) {
        if let Some(p) = &mut self.party {
            p.self_muted = muted;
        }
        self.emit_party();
    }

    fn set_deafened(&mut self, deafened: bool) {
        if let Some(p) = &mut self.party {
            p.self_deafened = deafened;
        }
        self.emit_party();
    }

    fn tick(&mut self) {}
}

/// Without the Discord SDK the app still compiles; these are never reached (no backend is created).
#[cfg(not(discord_sdk))]
impl DemoBackend {
    pub fn resume(&mut self, _refresh_token: &str) {}
    pub fn link(&mut self, _code: &str, _verifier: &str, _redirect: &str) {}
}
