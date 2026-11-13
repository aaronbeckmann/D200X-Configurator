//! Host-side library for the Ulanzi D200X: stock vendor HID protocol plus the
//! deckd configuration channel. Used by the CLI and the configurator GUI.

pub mod bridge;
pub mod device;
pub mod protocol;

pub use device::Deck;
pub use protocol::*;
