//! Akyba's terminals: a shell in a pty, alacritty's engine for the screen, a tap for Claude Code's
//! signals, and the GPUI view that draws it all.

pub mod keys;
pub mod palette;
pub mod pty;
pub mod signals;
pub mod terminal;

pub mod element;
pub mod view;
