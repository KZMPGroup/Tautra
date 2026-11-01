use std::collections::HashMap;
use std::sync::RwLock;

static STRINGS: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

#[derive(Clone, Copy)]
pub enum Key {
    Open,
    Disconnect,
    Resume,
    Gaming,
    Quit,
    AgentOffline,
    Starting,
    Idle,
    Searching,
    Connecting,
    Connected,
    Paused,
    Error,
    ToastConnectedTitle,
    ToastConnectedBody,
    ToastDisconnectedTitle,
    ToastDisconnectedBody,
    ToastPairedTitle,
    ToastPairedBody,
    ToastIncomingTitle,
    ToastOutgoingTitle,
    ToastTransferRunning,
    ToastTransferDoneTitle,
    ToastTransferDone,
    ToastTransferFailedTitle,
    ToastTransferFailed,
    ToastFailedTitle,
    ToastMoreFailures,
    ToastUpdatedTitle,
    ToastUpdatedBody,
    TrayTransfers,
}

impl Key {
    fn entry(self) -> (&'static str, &'static str) {
        use Key::*;
        match self {
            Open => ("open", "Open Tautra"),
            Disconnect => ("disconnect", "Disconnect"),
            Resume => ("resume", "Connect"),
            Gaming => ("gaming", "Gaming Mode"),
            Quit => ("quit", "Quit"),
            AgentOffline => ("agentOffline", "Agent is not running"),
            Starting => ("starting", "Starting…"),
            Idle => ("idle", "Idle"),
            Searching => ("searching", "Searching for devices…"),
            Connecting => ("connecting", "Connecting to {name}…"),
            Connected => ("connected", "Connected to {name}"),
            Paused => ("paused", "Paused"),
            Error => ("error", "Error"),
            ToastConnectedTitle => ("connectedTitle", "Connected"),
            ToastConnectedBody => ("connectedBody", "Connected to {name}"),
            ToastDisconnectedTitle => ("disconnectedTitle", "Disconnected"),
            ToastDisconnectedBody => ("disconnectedBody", "Connection to {name} was lost"),
            ToastPairedTitle => ("pairedTitle", "Pairing succeeded"),
            ToastPairedBody => ("pairedBody", "{name} is now a trusted device"),
            ToastIncomingTitle => ("incomingTitle", "Receiving files"),
            ToastOutgoingTitle => ("outgoingTitle", "Sending files"),
            ToastTransferRunning => (
                "transferRunning",
                "{name} is being transferred. Progress is on the Clipboard & files page.",
            ),
            ToastTransferDoneTitle => ("transferDoneTitle", "Transfer complete"),
            ToastTransferDone => ("transferDone", "{name} was transferred"),
            ToastTransferFailedTitle => ("transferFailedTitle", "Transfer failed"),
            ToastTransferFailed => ("transferFailed", "{name} could not be transferred"),
            ToastFailedTitle => ("failedTitle", "File could not be sent"),
            ToastMoreFailures => ("moreFailures", " (and {count} more failures)"),
            ToastUpdatedTitle => ("updatedTitle", "Tautra updated"),
            ToastUpdatedBody => (
                "updatedBody",
                "A newer version was received from the other device. Tautra is restarting.",
            ),
            TrayTransfers => ("transfers", "transfer {percent}"),
        }
    }
}

pub fn set(strings: HashMap<String, String>) -> bool {
    let mut current = STRINGS.write().unwrap_or_else(|e| e.into_inner());
    if current.as_ref() == Some(&strings) {
        return false;
    }
    *current = Some(strings);
    true
}

pub fn t(key: Key) -> String {
    let (name, english) = key.entry();
    STRINGS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|strings| strings.get(name))
        .filter(|text| !text.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| english.to_string())
}

pub fn with(key: Key, var: &str, value: &str) -> String {
    t(key).replace(&format!("{{{var}}}"), value)
}

pub fn with_name(key: Key, name: &str) -> String {
    with(key, "name", name)
}
