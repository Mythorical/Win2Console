//! Discord Social SDK backend: friends and presence, direct messages, and party voice calls (lobbies).
//!
//! FFI to the SDK's C API (`third_party/discord_social_sdk/include/cdiscord.h`), only the functions used
//! here. The SDK is only linked when it's present at build time (`cfg(discord_sdk)`, set by build.rs).
//! Callbacks fire inside `Discord_RunCallbacks`, which `tick()` calls on the UI thread.

#![cfg(discord_sdk)]

use std::ffi::c_void;
use std::rc::Rc;

use crate::social::{self, Event, Friend, Party, PartyMember, Presence, SocialBackend, Status};

#[allow(non_camel_case_types, non_snake_case, dead_code)]
mod ffi {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Discord_String {
        pub ptr: *mut u8,
        pub size: usize,
    }

    macro_rules! opaque {
        ($($name:ident),*) => {$(
            #[repr(C)]
            pub struct $name { pub opaque: *mut c_void }
            impl $name { pub fn empty() -> Self { Self { opaque: std::ptr::null_mut() } } }
        )*};
    }
    opaque!(
        Discord_AudioDevice,
        Discord_UserMessageSummary,
        Discord_Client,
        Discord_ClientResult,
        Discord_RelationshipHandle,
        Discord_UserHandle,
        Discord_Activity,
        Discord_MessageHandle,
        Discord_ActivityInvite,
        Discord_ActivityParty,
        Discord_ActivitySecrets
    );

    #[repr(C)]
    pub struct Discord_Call {
        pub opaque: [*mut c_void; 3],
    }

    #[repr(C)]
    pub struct Discord_RelationshipHandleSpan {
        pub ptr: *mut Discord_RelationshipHandle,
        pub size: usize,
    }

    #[repr(C)]
    pub struct Discord_AudioDeviceSpan {
        pub ptr: *mut Discord_AudioDevice,
        pub size: usize,
    }

    #[repr(C)]
    pub struct Discord_MessageHandleSpan {
        pub ptr: *mut Discord_MessageHandle,
        pub size: usize,
    }

    #[repr(C)]
    pub struct Discord_UserMessageSummarySpan {
        pub ptr: *mut Discord_UserMessageSummary,
        pub size: usize,
    }

    #[repr(C)]
    pub struct Discord_UInt64Span {
        pub ptr: *mut u64,
        pub size: usize,
    }

    pub type FreeFn = Option<unsafe extern "C" fn(*mut c_void)>;

    // Enum values from cdiscord.h.
    pub const STATUS_READY: i32 = 3;
    pub const STATUS_DISCONNECTED: i32 = 0;
    pub const TOKEN_BEARER: i32 = 1;
    pub const RELATIONSHIP_FRIEND: i32 = 1;
    pub const RELATIONSHIP_PENDING_INCOMING: i32 = 3;
    pub const USER_ONLINE: i32 = 0;
    pub const USER_IDLE: i32 = 3;
    pub const USER_DND: i32 = 4;
    pub const USER_STREAMING: i32 = 6;
    pub const AVATAR_PNG: i32 = 2;
    pub const ACTIVITY_PLAYING: i32 = 0;

    #[link(name = "discord_partner_sdk")]
    unsafe extern "C" {
        pub fn Discord_Free(ptr: *mut c_void);
        pub fn Discord_RunCallbacks();

        pub fn Discord_Client_Init(s: *mut Discord_Client);
        pub fn Discord_Client_Drop(s: *mut Discord_Client);
        pub fn Discord_Client_SetApplicationId(s: *mut Discord_Client, id: u64);
        pub fn Discord_Client_Connect(s: *mut Discord_Client);
        pub fn Discord_Client_Disconnect(s: *mut Discord_Client);
        pub fn Discord_Client_SetStatusChangedCallback(
            s: *mut Discord_Client,
            cb: unsafe extern "C" fn(status: i32, error: i32, detail: i32, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_GetDefaultCommunicationScopes(out: *mut Discord_String);
        pub fn Discord_Client_GetToken(
            s: *mut Discord_Client,
            app: u64,
            code: Discord_String,
            verifier: Discord_String,
            redirect: Discord_String,
            cb: TokenCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_RefreshToken(
            s: *mut Discord_Client,
            app: u64,
            refresh: Discord_String,
            cb: TokenCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_UpdateToken(
            s: *mut Discord_Client,
            kind: i32,
            token: Discord_String,
            cb: ResultCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_GetCurrentUserV2(s: *mut Discord_Client, out: *mut Discord_UserHandle) -> bool;
        pub fn Discord_Client_GetUser(s: *mut Discord_Client, id: u64, out: *mut Discord_UserHandle) -> bool;
        pub fn Discord_Client_GetRelationships(s: *mut Discord_Client, out: *mut Discord_RelationshipHandleSpan);
        pub fn Discord_RelationshipHandle_User(
            s: *mut Discord_RelationshipHandle,
            out: *mut Discord_UserHandle,
        ) -> bool;
        pub fn Discord_RelationshipHandle_DiscordRelationshipType(s: *mut Discord_RelationshipHandle) -> i32;
        pub fn Discord_RelationshipHandle_GameRelationshipType(s: *mut Discord_RelationshipHandle) -> i32;
        pub fn Discord_RelationshipHandle_Id(s: *mut Discord_RelationshipHandle) -> u64;
        pub fn Discord_UserHandle_Username(s: *mut Discord_UserHandle, out: *mut Discord_String);
        pub fn Discord_Client_SetRelationshipDeletedCallback(
            s: *mut Discord_Client,
            cb: unsafe extern "C" fn(user: u64, discord: bool, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_SetRelationshipGroupsUpdatedCallback(
            s: *mut Discord_Client,
            cb: unsafe extern "C" fn(user: u64, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_RelationshipHandle_Drop(s: *mut Discord_RelationshipHandle);
        pub fn Discord_UserHandle_Id(s: *mut Discord_UserHandle) -> u64;
        pub fn Discord_UserHandle_DisplayName(s: *mut Discord_UserHandle, out: *mut Discord_String);
        pub fn Discord_UserHandle_Status(s: *mut Discord_UserHandle) -> i32;
        pub fn Discord_UserHandle_GameActivity(s: *mut Discord_UserHandle, out: *mut Discord_Activity) -> bool;
        pub fn Discord_UserHandle_AvatarUrl(
            s: *mut Discord_UserHandle,
            animated: i32,
            stat: i32,
            out: *mut Discord_String,
        );
        pub fn Discord_UserHandle_Drop(s: *mut Discord_UserHandle);
        pub fn Discord_Client_SetRelationshipCreatedCallback(
            s: *mut Discord_Client,
            cb: unsafe extern "C" fn(user: u64, discord: bool, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_SetUserUpdatedCallback(
            s: *mut Discord_Client,
            cb: unsafe extern "C" fn(user: u64, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );

        pub fn Discord_Activity_Init(s: *mut Discord_Activity);
        pub fn Discord_Activity_Drop(s: *mut Discord_Activity);
        pub fn Discord_Activity_Name(s: *mut Discord_Activity, out: *mut Discord_String);
        pub fn Discord_Activity_SetType(s: *mut Discord_Activity, kind: i32);
        pub fn Discord_Activity_SetDetails(s: *mut Discord_Activity, value: *mut Discord_String);
        pub fn Discord_Activity_SetState(s: *mut Discord_Activity, value: *mut Discord_String);
        pub fn Discord_Activity_Details(s: *mut Discord_Activity, out: *mut Discord_String) -> bool;
        pub fn Discord_Activity_State(s: *mut Discord_Activity, out: *mut Discord_String) -> bool;
        pub fn Discord_Activity_SetParty(s: *mut Discord_Activity, value: *mut Discord_ActivityParty);
        pub fn Discord_Activity_SetSecrets(s: *mut Discord_Activity, value: *mut Discord_ActivitySecrets);
        pub fn Discord_ActivityParty_Init(s: *mut Discord_ActivityParty);
        pub fn Discord_ActivityParty_SetId(s: *mut Discord_ActivityParty, value: Discord_String);
        pub fn Discord_ActivityParty_SetCurrentSize(s: *mut Discord_ActivityParty, value: i32);
        pub fn Discord_ActivityParty_SetMaxSize(s: *mut Discord_ActivityParty, value: i32);
        pub fn Discord_ActivityParty_Drop(s: *mut Discord_ActivityParty);
        pub fn Discord_ActivitySecrets_Init(s: *mut Discord_ActivitySecrets);
        pub fn Discord_ActivitySecrets_SetJoin(s: *mut Discord_ActivitySecrets, value: Discord_String);
        pub fn Discord_ActivitySecrets_Drop(s: *mut Discord_ActivitySecrets);
        pub fn Discord_Client_UpdateRichPresence(
            s: *mut Discord_Client,
            activity: *mut Discord_Activity,
            cb: ResultCb,
            free: FreeFn,
            ud: *mut c_void,
        );

        pub fn Discord_Client_SetMessageCreatedCallback(
            s: *mut Discord_Client,
            cb: unsafe extern "C" fn(message: u64, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_GetMessageHandle(
            s: *mut Discord_Client,
            id: u64,
            out: *mut Discord_MessageHandle,
        ) -> bool;
        pub fn Discord_MessageHandle_Content(s: *mut Discord_MessageHandle, out: *mut Discord_String);
        pub fn Discord_MessageHandle_AuthorId(s: *mut Discord_MessageHandle) -> u64;
        pub fn Discord_MessageHandle_RecipientId(s: *mut Discord_MessageHandle) -> u64;
        pub fn Discord_MessageHandle_SentTimestamp(s: *mut Discord_MessageHandle) -> u64;
        pub fn Discord_MessageHandle_Drop(s: *mut Discord_MessageHandle);
        pub fn Discord_MessageHandle_Id(s: *mut Discord_MessageHandle) -> u64;
        pub fn Discord_MessageHandle_ChannelId(s: *mut Discord_MessageHandle) -> u64;
        pub fn Discord_Client_SendLobbyMessage(
            s: *mut Discord_Client,
            lobby: u64,
            content: Discord_String,
            cb: unsafe extern "C" fn(result: *mut Discord_ClientResult, message: u64, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_SendDiscordFriendRequest(
            s: *mut Discord_Client,
            username: Discord_String,
            cb: ResultCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_AcceptDiscordFriendRequest(
            s: *mut Discord_Client,
            user: u64,
            cb: ResultCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_RejectDiscordFriendRequest(
            s: *mut Discord_Client,
            user: u64,
            cb: ResultCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_GetInputDevices(s: *mut Discord_Client, cb: DevicesCb, free: FreeFn, ud: *mut c_void);
        pub fn Discord_Client_GetOutputDevices(s: *mut Discord_Client, cb: DevicesCb, free: FreeFn, ud: *mut c_void);
        pub fn Discord_Client_GetCurrentInputDevice(
            s: *mut Discord_Client,
            cb: DeviceCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_GetCurrentOutputDevice(
            s: *mut Discord_Client,
            cb: DeviceCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_SetInputDevice(
            s: *mut Discord_Client,
            id: Discord_String,
            cb: ResultCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_SetOutputDevice(
            s: *mut Discord_Client,
            id: Discord_String,
            cb: ResultCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_SetNoiseSuppression(s: *mut Discord_Client, on: bool);
        pub fn Discord_Client_SetInputVolume(s: *mut Discord_Client, volume: f32);
        pub fn Discord_Client_GetInputVolume(s: *mut Discord_Client) -> f32;
        pub fn Discord_AudioDevice_Id(s: *mut Discord_AudioDevice, out: *mut Discord_String);
        pub fn Discord_AudioDevice_Name(s: *mut Discord_AudioDevice, out: *mut Discord_String);
        pub fn Discord_Client_GetUserMessageSummaries(
            s: *mut Discord_Client,
            cb: unsafe extern "C" fn(
                result: *mut Discord_ClientResult,
                span: Discord_UserMessageSummarySpan,
                ud: *mut c_void,
            ),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_UserMessageSummary_UserId(s: *mut Discord_UserMessageSummary) -> u64;
        pub fn Discord_Client_GetUserMessagesWithLimit(
            s: *mut Discord_Client,
            recipient: u64,
            limit: i32,
            cb: unsafe extern "C" fn(
                result: *mut Discord_ClientResult,
                span: Discord_MessageHandleSpan,
                ud: *mut c_void,
            ),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_SendUserMessage(
            s: *mut Discord_Client,
            recipient: u64,
            content: Discord_String,
            cb: unsafe extern "C" fn(result: *mut Discord_ClientResult, message: u64, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );

        pub fn Discord_ClientResult_Successful(s: *mut Discord_ClientResult) -> bool;
        pub fn Discord_ClientResult_ToString(s: *mut Discord_ClientResult, out: *mut Discord_String);

        pub fn Discord_Client_CreateOrJoinLobby(
            s: *mut Discord_Client,
            secret: Discord_String,
            cb: unsafe extern "C" fn(result: *mut Discord_ClientResult, lobby: u64, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_LeaveLobby(
            s: *mut Discord_Client,
            lobby: u64,
            cb: ResultCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_StartCall(s: *mut Discord_Client, channel: u64, out: *mut Discord_Call) -> bool;
        pub fn Discord_Client_EndCall(
            s: *mut Discord_Client,
            channel: u64,
            cb: unsafe extern "C" fn(ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_GetOutputVolume(s: *mut Discord_Client) -> f32;
        pub fn Discord_Client_SetOutputVolume(s: *mut Discord_Client, volume: f32);
        pub fn Discord_Call_SetSelfMute(s: *mut Discord_Call, mute: bool);
        pub fn Discord_Call_SetSelfDeaf(s: *mut Discord_Call, deaf: bool);
        pub fn Discord_Call_Drop(s: *mut Discord_Call);
        pub fn Discord_Call_GetParticipants(s: *mut Discord_Call, out: *mut Discord_UInt64Span);
        pub fn Discord_Call_SetSpeakingStatusChangedCallback(
            s: *mut Discord_Call,
            cb: unsafe extern "C" fn(user: u64, speaking: bool, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Call_SetParticipantChangedCallback(
            s: *mut Discord_Call,
            cb: unsafe extern "C" fn(user: u64, added: bool, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );

        pub fn Discord_Client_SendActivityInvite(
            s: *mut Discord_Client,
            user: u64,
            content: Discord_String,
            cb: ResultCb,
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_SetActivityInviteCreatedCallback(
            s: *mut Discord_Client,
            cb: unsafe extern "C" fn(invite: *mut Discord_ActivityInvite, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_Client_AcceptActivityInvite(
            s: *mut Discord_Client,
            invite: *mut Discord_ActivityInvite,
            cb: unsafe extern "C" fn(result: *mut Discord_ClientResult, secret: Discord_String, ud: *mut c_void),
            free: FreeFn,
            ud: *mut c_void,
        );
        pub fn Discord_ActivityInvite_Clone(s: *mut Discord_ActivityInvite, rhs: *const Discord_ActivityInvite);
        pub fn Discord_ActivityInvite_SenderId(s: *mut Discord_ActivityInvite) -> u64;
        pub fn Discord_ActivityInvite_Drop(s: *mut Discord_ActivityInvite);
    }

    pub type TokenCb = unsafe extern "C" fn(
        result: *mut Discord_ClientResult,
        access: Discord_String,
        refresh: Discord_String,
        kind: i32,
        expires_in: i32,
        scopes: Discord_String,
        ud: *mut c_void,
    );
    pub type ResultCb = unsafe extern "C" fn(result: *mut Discord_ClientResult, ud: *mut c_void);
    pub type DevicesCb = unsafe extern "C" fn(devices: Discord_AudioDeviceSpan, ud: *mut c_void);
    pub type DeviceCb = unsafe extern "C" fn(device: *const Discord_AudioDevice, ud: *mut c_void);
}

use ffi::*;

/// Borrowed Rust text as an SDK string (the SDK copies it).
fn dstr(s: &str) -> Discord_String {
    Discord_String { ptr: s.as_ptr().cast_mut(), size: s.len() }
}

/// Reads an SDK-allocated string and frees it.
unsafe fn take(s: Discord_String) -> String {
    if s.ptr.is_null() {
        return String::new();
    }
    // SAFETY: the SDK returned `size` bytes at `ptr`, allocated by it; we copy, then free once.
    unsafe {
        let text = String::from_utf8_lossy(std::slice::from_raw_parts(s.ptr, s.size)).into_owned();
        Discord_Free(s.ptr.cast());
        text
    }
}

/// Reads a string the SDK passed to a callback (owned by the SDK; not freed).
unsafe fn borrowed(s: Discord_String) -> String {
    if s.ptr.is_null() {
        return String::new();
    }
    // SAFETY: valid for the duration of the callback.
    unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(s.ptr, s.size)).into_owned() }
}

fn result_ok(r: *mut Discord_ClientResult) -> Result<(), String> {
    // SAFETY: `r` is the result handle the SDK passed to the callback.
    unsafe {
        if r.is_null() || Discord_ClientResult_Successful(r) {
            return Ok(());
        }
        let mut s = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
        Discord_ClientResult_ToString(r, &mut s);
        Err(take(s))
    }
}

/// Default OAuth scopes for friends, presence, messaging and voice.
pub fn communication_scopes() -> String {
    let mut s = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
    // SAFETY: static function writing an SDK-allocated string.
    unsafe {
        Discord_Client_GetDefaultCommunicationScopes(&mut s);
        take(s)
    }
}

/// Things that happen inside SDK callbacks, handled by the backend after `RunCallbacks`.
enum Note {
    Status(i32, i32),
    Tokens {
        access: String,
        refresh: String,
    },
    TokenError(String),
    TokenApplied(Result<(), String>),
    FriendsChanged,
    Message(u64),
    Lobby(Result<u64, String>),
    Invite(Discord_ActivityInvite),
    JoinSecret(Result<String, String>),
    Speaking(u64, bool),
    RequestSent(Result<(), String>),
    Devices {
        input: bool,
        list: Vec<(String, String)>,
    },
    CurrentDevice {
        input: bool,
        id: String,
    },
    /// People with recent DM conversations (to load history for).
    Conversations(Vec<u64>),
    /// Loaded history: (id, author, recipient, text, sent ms).
    History(Vec<(u64, u64, u64, String, u64)>),
    ParticipantsChanged,
}

type Inbox = Rc<std::cell::RefCell<Vec<Note>>>;

/// Boxed callback context: the inbox the callback appends to.
fn ud(inbox: &Inbox) -> *mut c_void {
    Box::into_raw(Box::new(inbox.clone())).cast()
}

unsafe extern "C" fn free_ud(p: *mut c_void) {
    // SAFETY: created by `ud`; the SDK frees each callback context exactly once.
    drop(unsafe { Box::from_raw(p.cast::<Inbox>()) });
}

fn push(p: *mut c_void, note: Note) {
    // SAFETY: `p` is a live context from `ud` (the SDK keeps it until it calls `free_ud`).
    let inbox = unsafe { &*p.cast::<Inbox>() };
    inbox.borrow_mut().push(note);
}

unsafe extern "C" fn on_status(status: i32, error: i32, _detail: i32, p: *mut c_void) {
    push(p, Note::Status(status, error));
}
unsafe extern "C" fn on_tokens(
    r: *mut Discord_ClientResult,
    access: Discord_String,
    refresh: Discord_String,
    _kind: i32,
    _expires: i32,
    _scopes: Discord_String,
    p: *mut c_void,
) {
    match result_ok(r) {
        // SAFETY: strings valid during the callback.
        Ok(()) => push(p, unsafe { Note::Tokens { access: borrowed(access), refresh: borrowed(refresh) } }),
        Err(e) => push(p, Note::TokenError(e)),
    }
}
unsafe extern "C" fn on_token_applied(r: *mut Discord_ClientResult, p: *mut c_void) {
    push(p, Note::TokenApplied(result_ok(r)));
}
unsafe extern "C" fn on_ignore_result(_r: *mut Discord_ClientResult, _p: *mut c_void) {}
unsafe extern "C" fn on_ignore(_p: *mut c_void) {}
unsafe extern "C" fn on_relationship(_user: u64, _discord: bool, p: *mut c_void) {
    push(p, Note::FriendsChanged);
}
unsafe extern "C" fn on_user_updated(_user: u64, p: *mut c_void) {
    push(p, Note::FriendsChanged);
}
unsafe extern "C" fn on_message(id: u64, p: *mut c_void) {
    push(p, Note::Message(id));
}
unsafe extern "C" fn on_sent(r: *mut Discord_ClientResult, id: u64, p: *mut c_void) {
    if result_ok(r).is_ok() {
        push(p, Note::Message(id));
    }
}
unsafe extern "C" fn on_lobby(r: *mut Discord_ClientResult, lobby: u64, p: *mut c_void) {
    push(p, Note::Lobby(result_ok(r).map(|()| lobby)));
}
unsafe extern "C" fn on_invite(invite: *mut Discord_ActivityInvite, p: *mut c_void) {
    let mut copy = Discord_ActivityInvite::empty();
    // SAFETY: `invite` is valid during the callback; we keep our own clone.
    unsafe { Discord_ActivityInvite_Clone(&mut copy, invite) };
    push(p, Note::Invite(copy));
}
unsafe extern "C" fn on_join_secret(r: *mut Discord_ClientResult, secret: Discord_String, p: *mut c_void) {
    // SAFETY: string valid during the callback.
    push(p, Note::JoinSecret(result_ok(r).map(|()| unsafe { borrowed(secret) })));
}
unsafe extern "C" fn on_speaking(user: u64, speaking: bool, p: *mut c_void) {
    push(p, Note::Speaking(user, speaking));
}
unsafe extern "C" fn on_request_sent(r: *mut Discord_ClientResult, p: *mut c_void) {
    push(p, Note::RequestSent(result_ok(r)));
}
/// (id, name) of an audio device.
unsafe fn device(d: *mut Discord_AudioDevice) -> (String, String) {
    let mut id = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
    let mut name = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
    // SAFETY: valid device during the callback; strings are SDK-allocated and freed by `take`.
    unsafe {
        Discord_AudioDevice_Id(d, &mut id);
        Discord_AudioDevice_Name(d, &mut name);
        (take(id), take(name))
    }
}
unsafe fn devices(span: Discord_AudioDeviceSpan) -> Vec<(String, String)> {
    if span.ptr.is_null() {
        return Vec::new();
    }
    // SAFETY: the span is valid during the callback.
    unsafe { (0..span.size).map(|i| device(span.ptr.add(i))).collect() }
}
unsafe extern "C" fn on_inputs(span: Discord_AudioDeviceSpan, p: *mut c_void) {
    push(p, Note::Devices { input: true, list: unsafe { devices(span) } });
}
unsafe extern "C" fn on_outputs(span: Discord_AudioDeviceSpan, p: *mut c_void) {
    push(p, Note::Devices { input: false, list: unsafe { devices(span) } });
}
unsafe extern "C" fn on_current_input(d: *const Discord_AudioDevice, p: *mut c_void) {
    if !d.is_null() {
        push(p, Note::CurrentDevice { input: true, id: unsafe { device(d.cast_mut()) }.0 });
    }
}
unsafe extern "C" fn on_current_output(d: *const Discord_AudioDevice, p: *mut c_void) {
    if !d.is_null() {
        push(p, Note::CurrentDevice { input: false, id: unsafe { device(d.cast_mut()) }.0 });
    }
}
unsafe extern "C" fn on_summaries(r: *mut Discord_ClientResult, span: Discord_UserMessageSummarySpan, p: *mut c_void) {
    if result_ok(r).is_err() || span.ptr.is_null() {
        return;
    }
    // SAFETY: the span (owned by the SDK) is valid during the callback.
    let users = unsafe { (0..span.size).map(|i| Discord_UserMessageSummary_UserId(span.ptr.add(i))).collect() };
    push(p, Note::Conversations(users));
}
unsafe extern "C" fn on_history(r: *mut Discord_ClientResult, span: Discord_MessageHandleSpan, p: *mut c_void) {
    if result_ok(r).is_err() || span.ptr.is_null() {
        return;
    }
    // SAFETY: the span (owned by the SDK) is valid during the callback; strings we read are freed by `take`.
    let msgs = unsafe {
        (0..span.size)
            .map(|i| {
                let m = span.ptr.add(i);
                let mut content = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
                Discord_MessageHandle_Content(m, &mut content);
                (
                    Discord_MessageHandle_Id(m),
                    Discord_MessageHandle_AuthorId(m),
                    Discord_MessageHandle_RecipientId(m),
                    take(content),
                    Discord_MessageHandle_SentTimestamp(m),
                )
            })
            .collect()
    };
    push(p, Note::History(msgs));
}
unsafe extern "C" fn on_participant(_user: u64, _added: bool, p: *mut c_void) {
    push(p, Note::ParticipantsChanged);
}

/// What the backend wants persisted (the refresh token), sent through `on_tokens`.
pub type TokenSink = Box<dyn Fn(Option<String>)>;

pub struct DiscordBackend {
    client: Box<Discord_Client>,
    app_id: u64,
    inbox: Inbox,
    emit: Box<dyn Fn(Event)>,
    save_refresh_token: TokenSink,
    me: u64,
    lobby: Option<u64>,
    call: Option<Box<Discord_Call>>,
    party_secret: Option<String>,
    speaking: Vec<u64>,
    muted: bool,
    deafened: bool,
    invites: Vec<(u64, Discord_ActivityInvite)>,
    playing: Option<String>,
    audio: social::AudioInfo,
}

impl DiscordBackend {
    /// Creates the client. Call `link` with a fresh OAuth code, or `resume` with a saved refresh token.
    pub fn new(
        app_id: u64,
        emit: impl Fn(Event) + 'static,
        save_refresh_token: impl Fn(Option<String>) + 'static,
    ) -> Self {
        let mut client = Box::new(Discord_Client::empty());
        let inbox: Inbox = Rc::default();
        // SAFETY: `client` is heap-allocated so its address stays stable for the SDK.
        unsafe {
            Discord_Client_Init(&mut *client);
            Discord_Client_SetApplicationId(&mut *client, app_id);
            Discord_Client_SetStatusChangedCallback(&mut *client, on_status, Some(free_ud), ud(&inbox));
            Discord_Client_SetRelationshipCreatedCallback(&mut *client, on_relationship, Some(free_ud), ud(&inbox));
            Discord_Client_SetUserUpdatedCallback(&mut *client, on_user_updated, Some(free_ud), ud(&inbox));
            Discord_Client_SetRelationshipDeletedCallback(&mut *client, on_relationship, Some(free_ud), ud(&inbox));
            Discord_Client_SetRelationshipGroupsUpdatedCallback(
                &mut *client,
                on_user_updated,
                Some(free_ud),
                ud(&inbox),
            );
            Discord_Client_SetMessageCreatedCallback(&mut *client, on_message, Some(free_ud), ud(&inbox));
            Discord_Client_SetActivityInviteCreatedCallback(&mut *client, on_invite, Some(free_ud), ud(&inbox));
        }
        let backend = Self {
            client,
            app_id,
            inbox,
            emit: Box::new(emit),
            save_refresh_token: Box::new(save_refresh_token),
            me: 0,
            lobby: None,
            call: None,
            party_secret: None,
            speaking: Vec::new(),
            muted: false,
            deafened: false,
            invites: Vec::new(),
            playing: None,
            audio: social::AudioInfo { noise_suppression: true, mic_volume: 1.0, ..Default::default() },
        };
        (backend.emit)(Event::Status(Status::Connecting));
        backend
    }

    fn c(&mut self) -> *mut Discord_Client {
        &mut *self.client
    }

    /// Exchanges a fresh OAuth code (from the sign-in page) for tokens, then connects.
    pub fn link(&mut self, code: &str, verifier: &str, redirect: &str) {
        let (app, ud) = (self.app_id, ud(&self.inbox));
        // SAFETY: strings are copied by the SDK during the call.
        unsafe {
            Discord_Client_GetToken(
                self.c(),
                app,
                dstr(code),
                dstr(verifier),
                dstr(redirect),
                on_tokens,
                Some(free_ud),
                ud,
            )
        };
    }

    /// Signs in again with a saved refresh token.
    pub fn resume(&mut self, refresh_token: &str) {
        let (app, ud) = (self.app_id, ud(&self.inbox));
        // SAFETY: as in `link`.
        unsafe { Discord_Client_RefreshToken(self.c(), app, dstr(refresh_token), on_tokens, Some(free_ud), ud) };
    }

    fn handle(&mut self, note: Note) {
        match note {
            Note::Tokens { access, refresh } => {
                (self.save_refresh_token)(Some(refresh));
                let ud = ud(&self.inbox);
                // SAFETY: token copied by the SDK.
                unsafe {
                    Discord_Client_UpdateToken(
                        self.c(),
                        TOKEN_BEARER,
                        dstr(&access),
                        on_token_applied,
                        Some(free_ud),
                        ud,
                    )
                };
            }
            Note::TokenError(e) => {
                tracing::warn!("Discord sign-in failed: {e}");
                (self.save_refresh_token)(None);
                (self.emit)(Event::Status(Status::Failed("Sign in to Discord again".into())));
            }
            Note::TokenApplied(Ok(())) => {
                // SAFETY: plain call on our client.
                unsafe { Discord_Client_Connect(self.c()) };
            }
            Note::TokenApplied(Err(e)) => (self.emit)(Event::Status(Status::Failed(e))),
            Note::Status(status, error) => {
                if status == STATUS_READY {
                    let me = self.user(None);
                    self.me = me.as_ref().map_or(0, |f| f.id);
                    let name = me.map_or_else(|| "Discord".into(), |f| f.name);
                    (self.emit)(Event::Status(Status::Connected { user: name }));
                    self.publish_presence();
                    self.refresh_audio();
                    // Recent conversations (Discord keeps the last 72 hours / 200 messages per chat).
                    let ud = ud(&self.inbox);
                    // SAFETY: plain call on our client.
                    unsafe { Discord_Client_GetUserMessageSummaries(self.c(), on_summaries, Some(free_ud), ud) };
                    self.emit_friends();
                } else if status == STATUS_DISCONNECTED && error != 0 {
                    (self.emit)(Event::Status(Status::Failed(format!("Disconnected from Discord (error {error})"))));
                }
            }
            Note::FriendsChanged => self.emit_friends(),
            Note::Message(id) => self.emit_message(id),
            Note::Lobby(Ok(lobby)) => self.join_call(lobby),
            Note::Lobby(Err(e)) => tracing::warn!("Discord party: {e}"),
            Note::Invite(invite) => {
                // SAFETY: our own clone of the invite.
                let from = unsafe { Discord_ActivityInvite_SenderId(&invite as *const _ as *mut _) };
                let name = self.user(Some(from)).map_or_else(|| "A friend".into(), |f| f.name);
                self.invites.retain(|(id, _)| *id != from);
                self.invites.push((from, invite));
                (self.emit)(Event::PartyInvite { from, name });
            }
            Note::JoinSecret(Ok(secret)) => self.create_or_join(&secret),
            Note::JoinSecret(Err(e)) => tracing::warn!("Discord invite: {e}"),
            Note::Speaking(user, speaking) => {
                self.speaking.retain(|u| *u != user);
                if speaking {
                    self.speaking.push(user);
                }
                self.emit_party();
            }
            Note::ParticipantsChanged => self.emit_party(),
            Note::RequestSent(Ok(())) => (self.emit)(Event::Notice("Friend request sent".into())),
            Note::RequestSent(Err(e)) => (self.emit)(Event::Notice(format!("Couldn't send the friend request: {e}"))),
            Note::Devices { input, list } => {
                if input {
                    self.audio.inputs = list;
                } else {
                    self.audio.outputs = list;
                }
                (self.emit)(Event::Audio(self.audio.clone()));
            }
            Note::CurrentDevice { input, id } => {
                if input {
                    self.audio.input = Some(id);
                } else {
                    self.audio.output = Some(id);
                }
                (self.emit)(Event::Audio(self.audio.clone()));
            }
            Note::Conversations(users) => {
                for user in users {
                    self.emit_user(user);
                    self.load_history(user);
                }
            }
            Note::History(msgs) => {
                for (id, author, recipient, text, sent_ms) in msgs {
                    let from_me = author == self.me;
                    let peer = if from_me { recipient } else { author };
                    self.emit_user(peer);
                    let message = social::Message { id, from_me, text, at: (sent_ms / 1000) as i64 };
                    (self.emit)(Event::Message { peer, message });
                }
            }
        }
    }

    /// Tells the app who `id` is (for chats with people who aren't on the friends list).
    fn emit_user(&mut self, id: u64) {
        if id != 0
            && let Some(user) = self.user(Some(id))
        {
            (self.emit)(Event::User(user));
        }
    }

    /// A user as a `Friend` (`None` = the signed-in user).
    fn user(&mut self, id: Option<u64>) -> Option<Friend> {
        let mut u = Discord_UserHandle::empty();
        // SAFETY: the handle is filled by the SDK and dropped below.
        unsafe {
            let ok = match id {
                Some(id) => Discord_Client_GetUser(self.c(), id, &mut u),
                None => Discord_Client_GetCurrentUserV2(self.c(), &mut u),
            };
            if !ok {
                return None;
            }
            let friend = friend_from(&mut u);
            Discord_UserHandle_Drop(&mut u);
            Some(friend)
        }
    }

    fn emit_friends(&mut self) {
        let mut span = Discord_RelationshipHandleSpan { ptr: std::ptr::null_mut(), size: 0 };
        let mut friends = Vec::new();
        let mut requests = Vec::new();
        // SAFETY: the span is SDK-allocated; each handle is dropped, then the array freed.
        unsafe {
            Discord_Client_GetRelationships(self.c(), &mut span);
            for i in 0..span.size {
                let rel = span.ptr.add(i);
                let kind = Discord_RelationshipHandle_DiscordRelationshipType(rel);
                let game_kind = Discord_RelationshipHandle_GameRelationshipType(rel);
                let friend = kind == RELATIONSHIP_FRIEND || game_kind == RELATIONSHIP_FRIEND;
                if friend || kind == RELATIONSHIP_PENDING_INCOMING {
                    // The relationship doesn't always carry the user yet; look them up, and keep them even
                    // without details (they fill in when Discord sends a user update).
                    let id = Discord_RelationshipHandle_Id(rel);
                    let mut u = Discord_UserHandle::empty();
                    let f = if Discord_RelationshipHandle_User(rel, &mut u) {
                        let f = friend_from(&mut u);
                        Discord_UserHandle_Drop(&mut u);
                        Some(f)
                    } else {
                        self.user(Some(id))
                    };
                    let f = f.unwrap_or(Friend {
                        id,
                        name: String::new(),
                        avatar: None,
                        presence: Presence::Offline,
                        activity: None,
                    });
                    if friend { friends.push(f) } else { requests.push(f) }
                }
                Discord_RelationshipHandle_Drop(rel);
            }
            if !span.ptr.is_null() {
                Discord_Free(span.ptr.cast());
            }
        }
        (self.emit)(Event::Requests(requests));
        (self.emit)(Event::Friends(friends));
    }

    fn emit_message(&mut self, id: u64) {
        let mut m = Discord_MessageHandle::empty();
        // SAFETY: handle filled by the SDK and dropped below.
        unsafe {
            if !Discord_Client_GetMessageHandle(self.c(), id, &mut m) {
                return;
            }
            let mut content = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
            Discord_MessageHandle_Content(&mut m, &mut content);
            let text = take(content);
            let author = Discord_MessageHandle_AuthorId(&mut m);
            let recipient = Discord_MessageHandle_RecipientId(&mut m);
            let sent_ms = Discord_MessageHandle_SentTimestamp(&mut m);
            let channel = Discord_MessageHandle_ChannelId(&mut m);
            Discord_MessageHandle_Drop(&mut m);
            let from_me = author == self.me;
            // Party (lobby) messages go to the party's conversation, labelled with the sender.
            let (peer, text) = if self.lobby == Some(channel) {
                let name =
                    if from_me { String::new() } else { self.user(Some(author)).map_or_else(String::new, |f| f.name) };
                (channel, if name.is_empty() { text } else { format!("{name}: {text}") })
            } else {
                let peer = if from_me { recipient } else { author };
                self.emit_user(peer);
                (peer, text)
            };
            let message = social::Message { id, from_me, text, at: (sent_ms / 1000) as i64 };
            (self.emit)(Event::Message { peer, message });
        }
    }

    fn refresh_audio(&mut self) {
        // SAFETY: plain calls on our client; results arrive through callbacks.
        unsafe {
            Discord_Client_SetNoiseSuppression(self.c(), self.audio.noise_suppression);
            self.audio.mic_volume = (Discord_Client_GetInputVolume(self.c()) / 100.0).clamp(0.0, 1.0);
            Discord_Client_GetInputDevices(self.c(), on_inputs, Some(free_ud), ud(&self.inbox));
            Discord_Client_GetOutputDevices(self.c(), on_outputs, Some(free_ud), ud(&self.inbox));
            Discord_Client_GetCurrentInputDevice(self.c(), on_current_input, Some(free_ud), ud(&self.inbox));
            Discord_Client_GetCurrentOutputDevice(self.c(), on_current_output, Some(free_ud), ud(&self.inbox));
        }
    }

    fn create_or_join(&mut self, secret: &str) {
        self.party_secret = Some(secret.to_string());
        let ud = ud(&self.inbox);
        // SAFETY: secret copied by the SDK.
        unsafe { Discord_Client_CreateOrJoinLobby(self.c(), dstr(secret), on_lobby, Some(free_ud), ud) };
    }

    fn join_call(&mut self, lobby: u64) {
        self.lobby = Some(lobby);
        let mut call = Box::new(Discord_Call { opaque: [std::ptr::null_mut(); 3] });
        // SAFETY: call handle filled by the SDK; kept boxed until we end the call.
        unsafe {
            if Discord_Client_StartCall(self.c(), lobby, &mut *call) {
                Discord_Call_SetSpeakingStatusChangedCallback(&mut *call, on_speaking, Some(free_ud), ud(&self.inbox));
                Discord_Call_SetParticipantChangedCallback(&mut *call, on_participant, Some(free_ud), ud(&self.inbox));
                self.call = Some(call);
            }
        }
        self.advertise_party();
        self.emit_party();
    }

    /// Rich presence with the party's join secret, so friends can be invited to it.
    fn advertise_party(&mut self) {
        self.publish_presence();
    }

    /// Rich presence: the game being played (details) and, in a party, its join secret so friends can join.
    /// Other Win2Console users see this; Discord only shares an app's own presence with that app.
    fn publish_presence(&mut self) {
        let secret = self.party_secret.clone();
        let playing = self.playing.clone();
        // SAFETY: activity objects are initialised, filled, passed and dropped here; strings are copied.
        unsafe {
            let mut activity = Discord_Activity::empty();
            Discord_Activity_Init(&mut activity);
            Discord_Activity_SetType(&mut activity, ACTIVITY_PLAYING);
            if let Some(game) = &playing {
                let mut details = dstr(game);
                Discord_Activity_SetDetails(&mut activity, &mut details);
            }
            let mut state = dstr(if secret.is_some() { "In a party" } else { "On the dashboard" });
            Discord_Activity_SetState(&mut activity, &mut state);
            let Some(secret) = secret else {
                let ud = ud(&self.inbox);
                Discord_Client_UpdateRichPresence(self.c(), &mut activity, on_ignore_result, Some(free_ud), ud);
                Discord_Activity_Drop(&mut activity);
                return;
            };
            let mut party = Discord_ActivityParty::empty();
            Discord_ActivityParty_Init(&mut party);
            Discord_ActivityParty_SetId(&mut party, dstr(&secret));
            Discord_ActivityParty_SetCurrentSize(&mut party, self.participants().len().max(1) as i32);
            Discord_ActivityParty_SetMaxSize(&mut party, 8);
            Discord_Activity_SetParty(&mut activity, &mut party);
            let mut secrets = Discord_ActivitySecrets::empty();
            Discord_ActivitySecrets_Init(&mut secrets);
            Discord_ActivitySecrets_SetJoin(&mut secrets, dstr(&secret));
            Discord_Activity_SetSecrets(&mut activity, &mut secrets);
            let ud = ud(&self.inbox);
            Discord_Client_UpdateRichPresence(self.c(), &mut activity, on_ignore_result, Some(free_ud), ud);
            Discord_ActivitySecrets_Drop(&mut secrets);
            Discord_ActivityParty_Drop(&mut party);
            Discord_Activity_Drop(&mut activity);
        }
    }

    fn participants(&mut self) -> Vec<u64> {
        let Some(call) = self.call.as_mut() else { return Vec::new() };
        let mut span = Discord_UInt64Span { ptr: std::ptr::null_mut(), size: 0 };
        // SAFETY: SDK-allocated array, copied then freed.
        unsafe {
            Discord_Call_GetParticipants(&mut **call, &mut span);
            if span.ptr.is_null() {
                return Vec::new();
            }
            let ids = std::slice::from_raw_parts(span.ptr, span.size).to_vec();
            Discord_Free(span.ptr.cast());
            ids
        }
    }

    fn emit_party(&mut self) {
        if self.call.is_none() {
            (self.emit)(Event::Party(None));
            return;
        }
        let me = self.me;
        let ids = self.participants();
        let members = ids
            .into_iter()
            .filter(|id| *id != me)
            .map(|id| {
                let f = self.user(Some(id));
                PartyMember {
                    id,
                    name: f.as_ref().map_or_else(|| id.to_string(), |f| f.name.clone()),
                    avatar: f.and_then(|f| f.avatar),
                    speaking: self.speaking.contains(&id),
                    muted: false,
                }
            })
            .collect();
        let chat = self.lobby.unwrap_or(0);
        (self.emit)(Event::Party(Some(Party { chat, members, self_muted: self.muted, self_deafened: self.deafened })));
    }
}

/// Reads a user handle into a `Friend`.
unsafe fn friend_from(u: *mut Discord_UserHandle) -> Friend {
    // SAFETY: `u` is a valid handle owned by the caller.
    unsafe {
        let mut name = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
        Discord_UserHandle_DisplayName(u, &mut name);
        let mut name = take(name);
        if name.is_empty() {
            let mut username = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
            Discord_UserHandle_Username(u, &mut username);
            name = take(username);
        }
        let mut avatar = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
        Discord_UserHandle_AvatarUrl(u, AVATAR_PNG, AVATAR_PNG, &mut avatar);
        let avatar = take(avatar);
        let presence = match Discord_UserHandle_Status(u) {
            USER_ONLINE | USER_STREAMING => Presence::Online,
            USER_IDLE => Presence::Idle,
            USER_DND => Presence::DoNotDisturb,
            _ => Presence::Offline,
        };
        let mut activity = Discord_Activity::empty();
        // Win2Console's presence: details = the game, state = "In a party" / "On the dashboard".
        let playing = if Discord_UserHandle_GameActivity(u, &mut activity) {
            let mut game = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
            let mut state = Discord_String { ptr: std::ptr::null_mut(), size: 0 };
            let game = if Discord_Activity_Details(&mut activity, &mut game) { take(game) } else { String::new() };
            let state = if Discord_Activity_State(&mut activity, &mut state) { take(state) } else { String::new() };
            Discord_Activity_Drop(&mut activity);
            if !game.is_empty() { Some(format!("Playing {game}")) } else { Some(state).filter(|s| !s.is_empty()) }
        } else {
            None
        };
        Friend {
            id: Discord_UserHandle_Id(u),
            name,
            avatar: (!avatar.is_empty()).then(|| avatar.into()),
            presence,
            activity: playing,
        }
    }
}

impl SocialBackend for DiscordBackend {
    fn send_message(&mut self, peer: u64, text: &str) {
        let ud = ud(&self.inbox);
        if self.lobby == Some(peer) {
            // SAFETY: content copied by the SDK.
            unsafe { Discord_Client_SendLobbyMessage(self.c(), peer, dstr(text), on_sent, Some(free_ud), ud) };
            return;
        }
        // SAFETY: content copied by the SDK.
        unsafe { Discord_Client_SendUserMessage(self.c(), peer, dstr(text), on_sent, Some(free_ud), ud) };
    }

    fn invite_to_party(&mut self, peer: u64) {
        if self.party_secret.is_none() {
            let secret = format!("win2console-party-{}", uuid::Uuid::new_v4().simple());
            self.create_or_join(&secret);
        }
        let ud = ud(&self.inbox);
        // SAFETY: content copied by the SDK.
        unsafe {
            Discord_Client_SendActivityInvite(
                self.c(),
                peer,
                dstr("Join my party"),
                on_ignore_result,
                Some(free_ud),
                ud,
            )
        };
    }

    fn accept_party_invite(&mut self, from: u64) {
        let Some(pos) = self.invites.iter().position(|(id, _)| *id == from) else { return };
        let (_, mut invite) = self.invites.remove(pos);
        let ud = ud(&self.inbox);
        // SAFETY: our invite clone is valid; dropped after the call (the SDK copies what it needs).
        unsafe {
            Discord_Client_AcceptActivityInvite(self.c(), &mut invite, on_join_secret, Some(free_ud), ud);
            Discord_ActivityInvite_Drop(&mut invite);
        }
    }

    fn leave_party(&mut self) {
        if let Some(lobby) = self.lobby.take() {
            // SAFETY: plain calls on our client; the call handle is dropped once.
            unsafe {
                Discord_Client_EndCall(self.c(), lobby, on_ignore, Some(free_ud), ud(&self.inbox));
                Discord_Client_LeaveLobby(self.c(), lobby, on_ignore_result, Some(free_ud), ud(&self.inbox));
                if let Some(mut call) = self.call.take() {
                    Discord_Call_Drop(&mut *call);
                }
            }
        }
        self.party_secret = None;
        self.speaking.clear();
        (self.emit)(Event::Party(None));
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(call) = self.call.as_mut() {
            // SAFETY: live call handle.
            unsafe { Discord_Call_SetSelfMute(&mut **call, muted) };
        }
        self.emit_party();
    }

    fn set_deafened(&mut self, deafened: bool) {
        self.deafened = deafened;
        if let Some(call) = self.call.as_mut() {
            // SAFETY: live call handle.
            unsafe { Discord_Call_SetSelfDeaf(&mut **call, deafened) };
        }
        self.emit_party();
    }

    fn send_friend_request(&mut self, username: &str) {
        let ud = ud(&self.inbox);
        // SAFETY: username copied by the SDK.
        unsafe {
            Discord_Client_SendDiscordFriendRequest(self.c(), dstr(username.trim()), on_request_sent, Some(free_ud), ud)
        };
    }

    fn answer_friend_request(&mut self, from: u64, accept: bool) {
        let ud = ud(&self.inbox);
        // SAFETY: plain calls on our client.
        unsafe {
            if accept {
                Discord_Client_AcceptDiscordFriendRequest(self.c(), from, on_ignore_result, Some(free_ud), ud);
            } else {
                Discord_Client_RejectDiscordFriendRequest(self.c(), from, on_ignore_result, Some(free_ud), ud);
            }
        }
    }

    fn next_audio_device(&mut self, input: bool) {
        let (list, current) =
            if input { (&self.audio.inputs, &self.audio.input) } else { (&self.audio.outputs, &self.audio.output) };
        if list.is_empty() {
            return;
        }
        let pos = current.as_ref().and_then(|c| list.iter().position(|(id, _)| id == c));
        let next = list[pos.map_or(0, |p| (p + 1) % list.len())].0.clone();
        let ud = ud(&self.inbox);
        // SAFETY: id copied by the SDK.
        unsafe {
            if input {
                Discord_Client_SetInputDevice(self.c(), dstr(&next), on_ignore_result, Some(free_ud), ud);
            } else {
                Discord_Client_SetOutputDevice(self.c(), dstr(&next), on_ignore_result, Some(free_ud), ud);
            }
        }
        if input {
            self.audio.input = Some(next);
        } else {
            self.audio.output = Some(next);
        }
        (self.emit)(Event::Audio(self.audio.clone()));
    }

    fn set_noise_suppression(&mut self, on: bool) {
        self.audio.noise_suppression = on;
        // SAFETY: plain call on our client.
        unsafe { Discord_Client_SetNoiseSuppression(self.c(), on) };
        (self.emit)(Event::Audio(self.audio.clone()));
    }

    fn set_mic_volume(&mut self, volume: f32) {
        self.audio.mic_volume = volume.clamp(0.0, 1.0);
        // SAFETY: plain call on our client (the SDK's range is 0–100).
        unsafe { Discord_Client_SetInputVolume(self.c(), self.audio.mic_volume * 100.0) };
        (self.emit)(Event::Audio(self.audio.clone()));
    }

    fn load_history(&mut self, peer: u64) {
        let ud = ud(&self.inbox);
        // SAFETY: plain call on our client.
        unsafe { Discord_Client_GetUserMessagesWithLimit(self.c(), peer, 100, on_history, Some(free_ud), ud) };
    }

    fn set_playing(&mut self, game: Option<&str>) {
        self.playing = game.map(str::to_string);
        if self.me != 0 {
            self.publish_presence();
        }
    }

    fn output_volume(&mut self) -> Option<f32> {
        // SAFETY: plain call on our client. The SDK's range is 0–200 (100 = normal); we expose 0–100.
        Some((unsafe { Discord_Client_GetOutputVolume(self.c()) } / 100.0).clamp(0.0, 1.0))
    }

    fn set_output_volume(&mut self, volume: f32) {
        // SAFETY: plain call on our client.
        unsafe { Discord_Client_SetOutputVolume(self.c(), volume.clamp(0.0, 1.0) * 100.0) };
    }

    fn tick(&mut self) {
        // SAFETY: delivers queued SDK callbacks on this (UI) thread.
        unsafe { Discord_RunCallbacks() };
        let notes: Vec<Note> = std::mem::take(&mut *self.inbox.borrow_mut());
        for note in notes {
            self.handle(note);
        }
    }
}

impl Drop for DiscordBackend {
    fn drop(&mut self) {
        // SAFETY: tear down in order: call, invites, then the client (which frees callback contexts).
        unsafe {
            if let Some(mut call) = self.call.take() {
                Discord_Call_Drop(&mut *call);
            }
            for (_, mut invite) in self.invites.drain(..) {
                Discord_ActivityInvite_Drop(&mut invite);
            }
            Discord_Client_Disconnect(&mut *self.client);
            Discord_Client_Drop(&mut *self.client);
        }
    }
}
