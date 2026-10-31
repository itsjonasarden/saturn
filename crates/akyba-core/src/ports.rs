//! A block of ports for each checkout, so servers in parallel worktrees never ask for the same one.
//! Every terminal gets its checkout's first port as `AKYBA_PORT`; the nine after it are the
//! checkout's too. A repository uses them where it wants (`--port $AKYBA_PORT`); nothing changes
//! for one that doesn't. `SATURN_PORT`, its name before Akyba, is set too, for repositories that
//! read it.
//!
//! Blocks start well above the ports dev servers pick by themselves (3000, 5173, 8000…) and below the
//! range macOS hands out for outgoing connections. A checkout keeps its block for as long as it
//! exists, kept in a small file beside Akyba's settings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The first port of the first block.
pub const FIRST: u16 = 42_000;
/// Ports in each block.
pub const BLOCK: u16 = 10;
/// Blocks end before macOS's range for outgoing connections (49152 and up).
const END: u16 = 49_152;

/// Which checkout has which block, by its first port.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ports {
    blocks: BTreeMap<PathBuf, u16>,
}

impl Ports {
    /// Reads the blocks handed out so far; none when the file is missing or unreadable.
    pub fn load(file: &Path) -> Self {
        std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(file, serde_json::to_string_pretty(self).unwrap_or_default())
    }

    /// The first port of `checkout`'s block, handing it the lowest free one the first time. Blocks
    /// of checkouts that no longer exist (a removed worktree) are free again. `None` only when
    /// every block is taken.
    pub fn block_for(&mut self, checkout: &Path) -> Option<u16> {
        if let Some(port) = self.blocks.get(checkout) {
            return Some(*port);
        }
        self.blocks.retain(|path, _| path.exists());
        let taken: Vec<u16> = self.blocks.values().copied().collect();
        let port = (FIRST..END - BLOCK).step_by(usize::from(BLOCK)).find(|port| !taken.contains(port))?;
        self.blocks.insert(checkout.to_owned(), port);
        Some(port)
    }
}

/// Hands out `checkout`'s block from the file at `file`, saving a new one. The file is small and
/// read whole each time, so windows and restarts agree on it.
pub fn block_for(file: &Path, checkout: &Path) -> Option<u16> {
    let mut ports = Ports::load(file);
    let known = ports.clone();
    let port = ports.block_for(checkout)?;
    if ports != known {
        let _ = ports.save(file);
    }
    Some(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_checkout_keeps_its_own_block() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (main, worktree) = (dir.path().join("acme-web"), dir.path().join("acme-web-abc-123"));
        std::fs::create_dir_all(&main).expect("make folders");
        std::fs::create_dir_all(&worktree).expect("make folders");
        let mut ports = Ports::default();
        assert_eq!(ports.block_for(&main), Some(FIRST));
        assert_eq!(ports.block_for(&worktree), Some(FIRST + BLOCK));
        assert_eq!(ports.block_for(&main), Some(FIRST), "the same block every time");
    }

    #[test]
    fn a_removed_worktree_frees_its_block() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let [one, two, three] = ["a", "b", "c"].map(|name| dir.path().join(name));
        for folder in [&one, &two] {
            std::fs::create_dir_all(folder).expect("make folders");
        }
        let mut ports = Ports::default();
        ports.block_for(&one);
        ports.block_for(&two);
        std::fs::remove_dir(&one).expect("remove");
        std::fs::create_dir_all(&three).expect("make folders");
        assert_eq!(ports.block_for(&three), Some(FIRST), "the first block is free again");
        assert_eq!(ports.block_for(&two), Some(FIRST + BLOCK));
    }

    #[test]
    fn blocks_are_remembered_in_a_file() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let file = dir.path().join("data/ports.json");
        let (main, worktree) = (dir.path().join("acme-web"), dir.path().join("acme-web-abc-123"));
        std::fs::create_dir_all(&main).expect("make folders");
        std::fs::create_dir_all(&worktree).expect("make folders");
        assert_eq!(block_for(&file, &main), Some(FIRST));
        assert_eq!(block_for(&file, &worktree), Some(FIRST + BLOCK));
        assert_eq!(block_for(&file, &main), Some(FIRST));
        assert_eq!(Ports::load(&file).blocks.len(), 2);
    }
}
