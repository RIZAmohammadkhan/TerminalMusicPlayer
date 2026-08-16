//! Minimal MPRIS2 D-Bus server for external play/pause/stop control.
//!
//! Requires the `zbus` crate in `Cargo.toml`:
//! ```toml
//! zbus = "3"
//! ```

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::Connection;
use zbus::dbus_interface;

use crate::player::PlayState;

/// Commands sent from the MPRIS D-Bus thread to the main player loop.
#[derive(Debug, Clone, Copy)]
pub enum MprisCommand {
    Play,
    Pause,
    Stop,
    PlayPause,
    Next,
    Previous,
}

/// Shared playback state, readable from the MPRIS D-Bus thread.
#[derive(Clone)]
pub struct MprisState {
    inner: Arc<AtomicU8>,
}

impl MprisState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU8::new(0)),
        }
    }

    pub fn set(&self, state: PlayState) {
        self.inner.store(state as u8, Ordering::Relaxed);
    }
}

fn state_to_str(v: u8) -> &'static str {
    match v {
        1 => "Playing",
        2 => "Paused",
        _ => "Stopped",
    }
}

// ---------------------------------------------------------------------------
// org.mpris.MediaPlayer2  (root interface)
// ---------------------------------------------------------------------------

struct Root;

#[dbus_interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn quit(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }

    fn raise(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }

    #[dbus_interface(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[dbus_interface(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[dbus_interface(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[dbus_interface(property)]
    fn identity(&self) -> &str {
        "trix"
    }

    #[dbus_interface(property)]
    fn desktop_entry(&self) -> &str {
        "trix"
    }

    #[dbus_interface(property)]
    fn supported_uri_schemes(&self) -> Vec<&str> {
        vec!["file"]
    }

    #[dbus_interface(property)]
    fn supported_mime_types(&self) -> Vec<&str> {
        vec![
            "audio/mpeg",
            "audio/flac",
            "audio/ogg",
            "audio/x-wav",
            "audio/mp4",
        ]
    }
}

// ---------------------------------------------------------------------------
// org.mpris.MediaPlayer2.Player
// ---------------------------------------------------------------------------

struct PlayerInterface {
    tx: Mutex<Sender<MprisCommand>>,
    state: MprisState,
}

#[dbus_interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    fn next(&self) -> zbus::fdo::Result<()> {
        let _ = self.tx.lock().unwrap().send(MprisCommand::Next);
        Ok(())
    }

    fn previous(&self) -> zbus::fdo::Result<()> {
        let _ = self.tx.lock().unwrap().send(MprisCommand::Previous);
        Ok(())
    }

    fn play(&self) -> zbus::fdo::Result<()> {
        let _ = self.tx.lock().unwrap().send(MprisCommand::Play);
        Ok(())
    }

    fn pause(&self) -> zbus::fdo::Result<()> {
        let _ = self.tx.lock().unwrap().send(MprisCommand::Pause);
        Ok(())
    }

    fn stop(&self) -> zbus::fdo::Result<()> {
        let _ = self.tx.lock().unwrap().send(MprisCommand::Stop);
        Ok(())
    }

    fn play_pause(&self) -> zbus::fdo::Result<()> {
        let _ = self.tx.lock().unwrap().send(MprisCommand::PlayPause);
        Ok(())
    }

    #[dbus_interface(property)]
    fn playback_status(&self) -> String {
        state_to_str(self.state.inner.load(Ordering::Relaxed)).to_string()
    }

    #[dbus_interface(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[dbus_interface(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[dbus_interface(property)]
    fn can_stop(&self) -> bool {
        true
    }

    #[dbus_interface(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[dbus_interface(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[dbus_interface(property)]
    fn loop_status(&self) -> String {
        "None".to_string()
    }

    #[dbus_interface(property)]
    fn shuffle(&self) -> bool {
        false
    }

    #[dbus_interface(property)]
    fn volume(&self) -> f64 {
        1.0
    }
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

/// Spawn an MPRIS2 D-Bus server in a background thread.
///
/// The thread runs for the lifetime of the process.  Commands are delivered
/// through `tx`; `state` is polled whenever a D-Bus client queries the
/// `PlaybackStatus` property.
pub fn spawn_mpris_server(
    tx: Sender<MprisCommand>,
    state: MprisState,
) -> Result<thread::JoinHandle<()>> {
    let handle = thread::Builder::new()
        .name("trix-mpris".into())
        .spawn(move || {
            let conn = match Connection::session() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("trix: mpris: D-Bus session bus unavailable: {e}");
                    return;
                }
            };

            if let Err(e) = conn.object_server().at("/org/mpris/MediaPlayer2", Root) {
                eprintln!("trix: mpris: failed to register root interface: {e}");
                return;
            }

            let player_iface = PlayerInterface {
                tx: Mutex::new(tx),
                state,
            };

            if let Err(e) = conn
                .object_server()
                .at("/org/mpris/MediaPlayer2", player_iface)
            {
                eprintln!("trix: mpris: failed to register player interface: {e}");
                return;
            }

            // Request the well-known name.
            use zbus::fdo::RequestNameFlags;
            use zbus::names::WellKnownName;

            match DBusProxy::new(&conn) {
                Ok(proxy) => {
                    let well_known = match WellKnownName::try_from("org.mpris.MediaPlayer2.trix") {
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("trix: mpris: invalid well-known name: {e}");
                            return;
                        }
                    };
                    let flags = RequestNameFlags::AllowReplacement | RequestNameFlags::ReplaceExisting;
                    if let Err(e) = proxy.request_name(well_known, flags) {
                        eprintln!("trix: mpris: failed to request name: {e}");
                        return;
                    }
                }
                Err(e) => {
                    eprintln!("trix: mpris: failed to create DBus proxy: {e}");
                    return;
                }
            }

            // Keep the connection alive. zbus handles the object server in a background
            // thread automatically, so we just sleep until the process exits.
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        })?;

    Ok(handle)
}
