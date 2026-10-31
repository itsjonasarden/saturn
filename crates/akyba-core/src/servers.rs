//! The servers each terminal is running, and the ones a repository's Docker Compose runs for all of
//! them. A server started by an agent (`pnpm dev`, `vite`) runs somewhere under the terminal's
//! shell, so the processes under each shell that listen on a TCP port are that terminal's; a
//! container's ports belong to Docker, so those are read from `docker ps` instead.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A port something listens on, and a short name for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Server {
    pub port: u16,
    /// The app's folder in a monorepo (`web`, `api`), else the program (`vite`).
    pub name: String,
    /// The command line, for the tooltip.
    pub command: String,
}

/// A process: its parent and its command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    pub parent: u32,
    pub command: String,
}

/// `ps -A -o pid=,ppid=,args=`: every process by id.
pub fn parse_processes(output: &str) -> HashMap<u32, Process> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let parent = parts.next()?.parse().ok()?;
            Some((pid, Process { parent, command: parts.collect::<Vec<_>>().join(" ") }))
        })
        .collect()
}

/// `root` and every process under it.
pub fn descendants<S: std::hash::BuildHasher>(processes: &HashMap<u32, Process, S>, root: u32) -> HashSet<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, process) in processes {
        children.entry(process.parent).or_default().push(*pid);
    }
    let mut found = HashSet::from([root]);
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        for child in children.get(&pid).into_iter().flatten() {
            if found.insert(*child) {
                stack.push(*child);
            }
        }
    }
    found
}

/// `lsof -F pn` output: each process's `p<pid>`, then an `n<name>` per open file. Names are
/// addresses (`*:5173`, `[::1]:3000`) or, with `-d cwd`, folders. Pairs each name with its process.
pub fn parse_lsof(output: &str) -> Vec<(u32, String)> {
    let mut pid = None;
    let mut found = Vec::new();
    for line in output.lines() {
        if let Some(id) = line.strip_prefix('p') {
            pid = id.parse().ok();
        } else if let (Some(name), Some(pid)) = (line.strip_prefix('n'), pid) {
            found.push((pid, name.to_owned()));
        }
    }
    found
}

/// The port at the end of an address such as `*:5173`, `127.0.0.1:3000` or `[::1]:4000`.
pub fn port_of(address: &str) -> Option<u16> {
    address.rsplit_once(':')?.1.parse().ok()
}

/// Runtimes and package managers that start a server, passed over when naming it.
const STARTERS: [&str; 19] = [
    "node", "bun", "deno", "python", "python3", "ruby", "php", "sh", "bash", "zsh", "env", "npx", "pnpm", "npm",
    "yarn", "uv", "bundle", "exec", "run",
];

/// What to call a server: its folder when that's an app inside the checkout (`apps/web` gives
/// `web`), else the program it runs, past the runtimes and package managers that start it.
pub fn name_for(command: &str, folder: Option<&Path>, checkout: Option<&Path>) -> String {
    if let (Some(folder), Some(checkout)) = (folder, checkout)
        && folder != checkout
        && folder.starts_with(checkout)
        && let Some(name) = folder.file_name()
    {
        return name.to_string_lossy().into_owned();
    }
    let program = |word: &str| {
        let base = word.rsplit('/').next().unwrap_or(word);
        base.split_once('.').map_or(base, |(stem, _)| stem).to_owned()
    };
    command
        .split_whitespace()
        .filter(|word| !word.starts_with('-') && !word.contains('='))
        .map(program)
        .find(|name| !name.is_empty() && !STARTERS.contains(&name.to_lowercase().as_str()))
        .or_else(|| command.split_whitespace().next().map(program))
        .unwrap_or_default()
}

/// A Docker Compose service with ports on this Mac, from
/// `docker ps --format '{{.Label "com.docker.compose.service"}}\t{{.Label "com.docker.compose.project.working_dir"}}\t{{.Ports}}'`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    /// The folder `docker compose up` ran in.
    pub folder: PathBuf,
    pub ports: Vec<u16>,
}

pub fn parse_docker(output: &str) -> Vec<Service> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let (name, folder, ports) = (fields.next()?, fields.next()?, fields.next()?);
            // `0.0.0.0:5432->5432/tcp, [::]:5432->5432/tcp`: the host's side, once each.
            let mut ports: Vec<u16> = ports
                .split(", ")
                .filter_map(|mapping| mapping.split_once("->").and_then(|(host, _)| port_of(host)))
                .collect();
            ports.sort_unstable();
            ports.dedup();
            (!name.is_empty() && !folder.is_empty() && !ports.is_empty()).then(|| Service {
                name: name.to_owned(),
                folder: PathBuf::from(folder),
                ports,
            })
        })
        .collect()
}

/// The servers under each shell, by the shell's process id, each named against its terminal's
/// checkout. Runs `ps` and `lsof`: off the main thread.
pub fn read(shells: &[(u32, Option<PathBuf>)]) -> HashMap<u32, Vec<Server>> {
    let Some(processes) = run("ps", &["-A", "-o", "pid=,ppid=,args="]).map(|out| parse_processes(&out)) else {
        return HashMap::new();
    };
    let owner: HashMap<u32, u32> = shells
        .iter()
        .flat_map(|(shell, _)| descendants(&processes, *shell).into_iter().map(|pid| (pid, *shell)))
        .collect();
    if owner.is_empty() {
        return HashMap::new();
    }
    let pids = owner.keys().map(u32::to_string).collect::<Vec<_>>().join(",");
    let listening = run("lsof", &["-nP", "-a", "-p", &pids, "-iTCP", "-sTCP:LISTEN", "-Fpn"])
        .map(|out| parse_lsof(&out))
        .unwrap_or_default();
    if listening.is_empty() {
        return HashMap::new();
    }
    let listeners: Vec<String> = listening.iter().map(|(pid, _)| pid.to_string()).collect();
    let folders: HashMap<u32, PathBuf> = run("lsof", &["-nP", "-a", "-p", &listeners.join(","), "-d", "cwd", "-Fpn"])
        .map(|out| parse_lsof(&out).into_iter().map(|(pid, folder)| (pid, PathBuf::from(folder))).collect())
        .unwrap_or_default();
    let checkouts: HashMap<u32, &Path> =
        shells.iter().filter_map(|(shell, checkout)| Some((*shell, checkout.as_deref()?))).collect();

    // One server per port and shell: a server listening on both IPv4 and IPv6 is still one.
    let mut found: HashMap<u32, BTreeMap<u16, Server>> = HashMap::new();
    for (pid, address) in listening {
        let (Some(port), Some(shell)) = (port_of(&address), owner.get(&pid)) else { continue };
        let command = processes.get(&pid).map(|process| process.command.clone()).unwrap_or_default();
        let name = name_for(&command, folders.get(&pid).map(PathBuf::as_path), checkouts.get(shell).copied());
        found.entry(*shell).or_default().entry(port).or_insert(Server { port, name, command });
    }
    found.into_iter().map(|(shell, servers)| (shell, servers.into_values().collect())).collect()
}

/// The Compose services running with ports on this Mac, when Docker is there and running.
pub fn docker_services() -> Vec<Service> {
    let format =
        "{{.Label \"com.docker.compose.service\"}}\t{{.Label \"com.docker.compose.project.working_dir\"}}\t{{.Ports}}";
    run("docker", &["ps", "--format", format]).map(|out| parse_docker(&out)).unwrap_or_default()
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_everything_under_a_shell() {
        let processes = parse_processes(
            "  100     1 -zsh\n  200   100 claude\n  300   200 /bin/zsh -c pnpm dev\n  400   300 node vite.js\n  500     1 other\n",
        );
        assert_eq!(descendants(&processes, 100), HashSet::from([100, 200, 300, 400]));
        assert_eq!(processes[&400].command, "node vite.js");
    }

    #[test]
    fn reads_listening_ports_once_per_process() {
        let pairs = parse_lsof("p400\nf20\nn*:5173\nf21\nn[::1]:5173\np410\nf9\nn127.0.0.1:3000\n");
        assert_eq!(
            pairs.iter().map(|(pid, name)| (*pid, port_of(name))).collect::<Vec<_>>(),
            [(400, Some(5173)), (400, Some(5173)), (410, Some(3000))]
        );
    }

    #[test]
    fn names_a_server_after_its_app_or_its_program() {
        let repo = Path::new("/code/acme-web");
        let web = repo.join("apps/web");
        assert_eq!(name_for("node /x/node_modules/vite/bin/vite.js", Some(&web), Some(repo)), "web");
        assert_eq!(name_for("node /x/node_modules/vite/bin/vite.js", Some(repo), Some(repo)), "vite");
        assert_eq!(name_for("node --enable-source-maps dist/main", None, None), "main");
        assert_eq!(name_for("/usr/bin/python3 -m http.server 8000", None, None), "http");
        assert_eq!(name_for("bundle exec rails server", None, None), "rails");
        assert_eq!(
            name_for(
                "/Library/Frameworks/Python.framework/Versions/3.12/Resources/Python.app/Contents/MacOS/Python -m http.server 8000",
                None,
                None
            ),
            "http"
        );
    }

    #[test]
    fn reads_compose_services_with_ports_on_this_mac() {
        let services = parse_docker(
            "postgres\t/code/acme-web\t0.0.0.0:5432->5432/tcp, [::]:5432->5432/tcp\nworker\t/code/acme-web\t\n\t\t0.0.0.0:80->80/tcp\n",
        );
        assert_eq!(services, [Service { name: "postgres".into(), folder: "/code/acme-web".into(), ports: vec![5432] }]);
    }
}
