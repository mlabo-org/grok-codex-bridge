//! Safe quiescence and relaunch boundary for ChatGPT.app mode transitions.
//!
//! This module deliberately owns no configuration or service mutation.  The
//! caller supplies the mutation closure, which is invoked only after both the
//! desktop application and its bundled app-server have disappeared.

use std::io;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

const OPEN: &str = "/usr/bin/open";
const KILL: &str = "/bin/kill";
const PS: &str = "/bin/ps";
const CHATGPT_APP: &str = "/Applications/ChatGPT.app";
const CHATGPT_EXECUTABLE: &str = "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT";
const CODEX_EXECUTABLE: &str = "/Applications/ChatGPT.app/Contents/Resources/codex";
const MAX_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Operations required by the transition.  The trait keeps the process
/// boundary testable without allowing tests to perform real app operations.
pub trait DesktopTransitionOperations {
    fn app_pids(&mut self) -> io::Result<Vec<u32>>;
    fn app_server_pids(&mut self) -> io::Result<Vec<u32>>;
    fn request_graceful_quit(
        &mut self,
        app_pids: &[u32],
        app_server_pids: &[u32],
    ) -> io::Result<()>;
    fn captured_pids_running(
        &mut self,
        app_pids: &[u32],
        app_server_pids: &[u32],
    ) -> io::Result<bool>;
    fn launch_app(&mut self) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopTransitionStage {
    InspectInitialState,
    RequestGracefulQuit,
    WaitForQuiescence,
    Mutate,
    Relaunch,
}

impl std::fmt::Display for DesktopTransitionStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InspectInitialState => "inspect initial desktop state",
            Self::RequestGracefulQuit => "request graceful ChatGPT.app quit",
            Self::WaitForQuiescence => "wait for ChatGPT.app and app-server quiescence",
            Self::Mutate => "mode mutation",
            Self::Relaunch => "relaunch ChatGPT.app",
        })
    }
}

#[derive(Debug, Error)]
pub enum DesktopTransitionError {
    #[error("desktop transition failed during {stage}: {source}")]
    Stage {
        stage: DesktopTransitionStage,
        #[source]
        source: io::Error,
    },
    #[error("desktop transition failed during {stage}: {message}")]
    Mutation {
        stage: DesktopTransitionStage,
        message: String,
    },
    #[error("ChatGPT.app and bundled app-server did not quiesce before the deadline")]
    QuiescenceTimeout,
}

/// Run one transition around `mutation`.
///
/// If either process was present at entry, the application is relaunched only
/// after a successful mutation.  A timeout or mutation failure never relaunches
/// the application, preventing a half-switched configuration from being read.
pub fn transition<F, E>(
    timeout: Duration,
    poll_interval: Duration,
    mutation: F,
) -> Result<(), DesktopTransitionError>
where
    F: FnOnce() -> Result<(), E>,
    E: std::fmt::Display,
{
    let mut operations = SystemDesktopTransitionOperations;
    transition_with_operations(&mut operations, timeout, poll_interval, mutation)
}

pub fn transition_with_operations<O, F, E>(
    operations: &mut O,
    timeout: Duration,
    poll_interval: Duration,
    mutation: F,
) -> Result<(), DesktopTransitionError>
where
    O: DesktopTransitionOperations,
    F: FnOnce() -> Result<(), E>,
    E: std::fmt::Display,
{
    let app_pids = operations
        .app_pids()
        .map_err(|source| DesktopTransitionError::Stage {
            stage: DesktopTransitionStage::InspectInitialState,
            source,
        })?;
    let app_server_pids =
        operations
            .app_server_pids()
            .map_err(|source| DesktopTransitionError::Stage {
                stage: DesktopTransitionStage::InspectInitialState,
                source,
            })?;
    let relaunch = !app_pids.is_empty() || !app_server_pids.is_empty();

    if relaunch {
        operations
            .request_graceful_quit(&app_pids, &app_server_pids)
            .map_err(|source| DesktopTransitionError::Stage {
                stage: DesktopTransitionStage::RequestGracefulQuit,
                source,
            })?;
        let deadline = Instant::now() + timeout;
        let interval = poll_interval.min(MAX_POLL_INTERVAL);
        loop {
            let captured_running = operations
                .captured_pids_running(&app_pids, &app_server_pids)
                .map_err(|source| DesktopTransitionError::Stage {
                    stage: DesktopTransitionStage::WaitForQuiescence,
                    source,
                })?;
            if !captured_running {
                break;
            }
            if Instant::now() >= deadline {
                return Err(DesktopTransitionError::QuiescenceTimeout);
            }
            thread::sleep(interval);
        }
    }

    mutation().map_err(|error| DesktopTransitionError::Mutation {
        stage: DesktopTransitionStage::Mutate,
        message: error.to_string(),
    })?;

    if relaunch {
        operations
            .launch_app()
            .map_err(|source| DesktopTransitionError::Stage {
                stage: DesktopTransitionStage::Relaunch,
                source,
            })?;
    }
    Ok(())
}

struct SystemDesktopTransitionOperations;

impl DesktopTransitionOperations for SystemDesktopTransitionOperations {
    fn app_pids(&mut self) -> io::Result<Vec<u32>> {
        process_pids(is_chatgpt_command)
    }

    fn app_server_pids(&mut self) -> io::Result<Vec<u32>> {
        process_pids(is_app_server_command)
    }

    fn request_graceful_quit(
        &mut self,
        app_pids: &[u32],
        app_server_pids: &[u32],
    ) -> io::Result<()> {
        terminate_captured(app_pids, is_chatgpt_command)?;
        terminate_captured(app_server_pids, is_app_server_command)
    }

    fn captured_pids_running(
        &mut self,
        app_pids: &[u32],
        app_server_pids: &[u32],
    ) -> io::Result<bool> {
        Ok(captured_pids_running(app_pids, is_chatgpt_command)?
            || captured_pids_running(app_server_pids, is_app_server_command)?)
    }

    fn launch_app(&mut self) -> io::Result<()> {
        let output = Command::new(OPEN).args(["-a", CHATGPT_APP]).output()?;
        ensure_success(output, "open ChatGPT.app")
    }
}

fn is_chatgpt_command(command: &str) -> bool {
    command == CHATGPT_EXECUTABLE || command.starts_with(&format!("{CHATGPT_EXECUTABLE} "))
}

fn is_app_server_command(command: &str) -> bool {
    let mut arguments = command.split_ascii_whitespace();
    if arguments.next() != Some(CODEX_EXECUTABLE) {
        return false;
    }
    while let Some(argument) = arguments.next() {
        match argument {
            "-c" | "--config" => {
                let Some(value) = arguments.next() else {
                    return false;
                };
                if !is_config_override(value) {
                    return false;
                }
            }
            value if value.starts_with("--config=") => {
                if !is_config_override(&value["--config=".len()..]) {
                    return false;
                }
            }
            "app-server" => return true,
            _ => return false,
        }
    }
    false
}

fn is_config_override(value: &str) -> bool {
    value
        .split_once('=')
        .is_some_and(|(key, _)| !key.is_empty())
}

fn process_pids(predicate: impl Fn(&str) -> bool) -> io::Result<Vec<u32>> {
    let output = Command::new(PS).args(["-axo", "pid=,command="]).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "ps process inspection exited with {}",
            output.status
        )));
    }
    let commands = std::str::from_utf8(&output.stdout)
        .map_err(|_| io::Error::other("ps returned non-UTF-8 process data"))?;
    Ok(commands
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim().split_once(char::is_whitespace)?;
            let pid = pid.trim().parse().ok()?;
            predicate(command.trim()).then_some(pid)
        })
        .collect())
}

fn current_pid_command(pid: u32) -> io::Result<Option<String>> {
    let output = Command::new(PS)
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(std::str::from_utf8(&output.stdout)
        .ok()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(ToOwned::to_owned))
}

fn captured_pids_running(pids: &[u32], predicate: impl Fn(&str) -> bool) -> io::Result<bool> {
    for pid in pids {
        if current_pid_command(*pid)?.is_some_and(|command| predicate(&command)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn terminate_captured(pids: &[u32], predicate: impl Fn(&str) -> bool) -> io::Result<()> {
    for pid in pids {
        // Re-read immediately before TERM so a reused PID cannot be signalled.
        if current_pid_command(*pid)?.is_some_and(|command| predicate(&command)) {
            let status = Command::new(KILL)
                .args(["-TERM", &pid.to_string()])
                .status()?;
            if !status.success()
                && current_pid_command(*pid)?.is_some_and(|command| predicate(&command))
            {
                return Err(io::Error::other(format!(
                    "TERM for captured PID {pid} exited with {status}"
                )));
            }
        }
    }
    Ok(())
}

fn ensure_success(output: Output, operation: &str) -> io::Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{operation} exited with {}",
            output.status
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    #[derive(Debug)]
    struct Fake {
        app: VecDeque<Vec<u32>>,
        server: VecDeque<Vec<u32>>,
        current: HashMap<u32, &'static str>,
        term_signals: Vec<u32>,
        events: Vec<&'static str>,
        launch_error: bool,
        terminate_on_quit: bool,
    }

    impl Fake {
        fn next(queue: &mut VecDeque<Vec<u32>>) -> Vec<u32> {
            let current = queue.front().cloned().unwrap_or_default();
            if queue.len() > 1 {
                queue.pop_front();
            }
            current
        }
    }

    impl DesktopTransitionOperations for Fake {
        fn app_pids(&mut self) -> io::Result<Vec<u32>> {
            self.events.push("app_pids");
            Ok(Self::next(&mut self.app))
        }
        fn app_server_pids(&mut self) -> io::Result<Vec<u32>> {
            self.events.push("server_pids");
            Ok(Self::next(&mut self.server))
        }
        fn request_graceful_quit(
            &mut self,
            app_pids: &[u32],
            app_server_pids: &[u32],
        ) -> io::Result<()> {
            self.events.push("quit");
            for pid in app_pids {
                if self
                    .current
                    .get(pid)
                    .is_some_and(|command| is_chatgpt_command(command))
                {
                    self.term_signals.push(*pid);
                    if self.terminate_on_quit {
                        self.current.remove(pid);
                    }
                }
            }
            for pid in app_server_pids {
                if self
                    .current
                    .get(pid)
                    .is_some_and(|command| is_app_server_command(command))
                {
                    self.term_signals.push(*pid);
                    if self.terminate_on_quit {
                        self.current.remove(pid);
                    }
                }
            }
            Ok(())
        }
        fn captured_pids_running(
            &mut self,
            app_pids: &[u32],
            app_server_pids: &[u32],
        ) -> io::Result<bool> {
            self.events.push("captured_status");
            Ok(app_pids.iter().any(|pid| {
                self.current
                    .get(pid)
                    .is_some_and(|command| is_chatgpt_command(command))
            }) || app_server_pids.iter().any(|pid| {
                self.current
                    .get(pid)
                    .is_some_and(|command| is_app_server_command(command))
            }))
        }
        fn launch_app(&mut self) -> io::Result<()> {
            self.events.push("launch");
            if self.launch_error {
                Err(io::Error::other("launch failed"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn success_mutates_between_quit_and_relaunch() {
        let mut fake = Fake {
            app: [vec![11], vec![]].into(),
            server: [vec![12], vec![]].into(),
            current: [
                (11, CHATGPT_EXECUTABLE),
                (
                    12,
                    "/Applications/ChatGPT.app/Contents/Resources/codex app-server",
                ),
            ]
            .into_iter()
            .collect(),
            term_signals: Vec::new(),
            events: Vec::new(),
            launch_error: false,
            terminate_on_quit: true,
        };
        let mut mutated = false;
        transition_with_operations(&mut fake, Duration::from_secs(1), Duration::ZERO, || {
            mutated = true;
            Ok::<_, io::Error>(())
        })
        .unwrap();
        assert!(mutated);
        assert_eq!(
            fake.events,
            [
                "app_pids",
                "server_pids",
                "quit",
                "captured_status",
                "launch"
            ]
        );
        assert_eq!(fake.term_signals, [11, 12]);
    }

    #[test]
    fn timeout_does_not_mutate_or_relaunch() {
        let mut fake = Fake {
            app: [vec![11]].into(),
            server: [vec![12]].into(),
            current: [
                (11, CHATGPT_EXECUTABLE),
                (
                    12,
                    "/Applications/ChatGPT.app/Contents/Resources/codex app-server",
                ),
            ]
            .into_iter()
            .collect(),
            term_signals: Vec::new(),
            events: Vec::new(),
            launch_error: false,
            terminate_on_quit: false,
        };
        let mut mutated = false;
        let result = transition_with_operations(&mut fake, Duration::ZERO, Duration::ZERO, || {
            mutated = true;
            Ok::<_, io::Error>(())
        });
        assert!(matches!(
            result,
            Err(DesktopTransitionError::QuiescenceTimeout)
        ));
        assert!(!mutated);
        assert!(!fake.events.contains(&"launch"));
    }

    #[test]
    fn mutation_failure_does_not_relaunch() {
        let mut fake = Fake {
            app: [vec![11], vec![]].into(),
            server: [vec![], vec![]].into(),
            current: [(11, CHATGPT_EXECUTABLE)].into_iter().collect(),
            term_signals: Vec::new(),
            events: Vec::new(),
            launch_error: false,
            terminate_on_quit: true,
        };
        let result =
            transition_with_operations(&mut fake, Duration::from_secs(1), Duration::ZERO, || {
                Err::<(), _>("mutation failed")
            });
        assert!(matches!(
            result,
            Err(DesktopTransitionError::Mutation { .. })
        ));
        assert!(!fake.events.contains(&"launch"));
    }

    #[test]
    fn initially_stopped_app_is_not_relaunched() {
        let mut fake = Fake {
            app: [vec![]].into(),
            server: [vec![]].into(),
            current: HashMap::new(),
            term_signals: Vec::new(),
            events: Vec::new(),
            launch_error: false,
            terminate_on_quit: true,
        };
        transition_with_operations(&mut fake, Duration::ZERO, Duration::ZERO, || {
            Ok::<_, io::Error>(())
        })
        .unwrap();
        assert!(!fake.events.contains(&"quit"));
        assert!(!fake.events.contains(&"launch"));
    }

    #[test]
    fn replaced_captured_pid_is_not_signalled_or_kept_running() {
        let mut fake = Fake {
            app: [vec![11]].into(),
            server: [vec![]].into(),
            current: [(11, "/usr/libexec/other-process")].into_iter().collect(),
            term_signals: Vec::new(),
            events: Vec::new(),
            launch_error: false,
            terminate_on_quit: true,
        };
        transition_with_operations(&mut fake, Duration::from_secs(1), Duration::ZERO, || {
            Ok::<_, io::Error>(())
        })
        .unwrap();
        assert!(fake.term_signals.is_empty());
        assert!(fake.events.contains(&"launch"));
    }

    #[test]
    fn process_classification_matches_the_current_bundled_commands_only() {
        assert!(is_chatgpt_command(
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
        ));
        assert!(is_app_server_command(
            "/Applications/ChatGPT.app/Contents/Resources/codex -c features.code_mode_host=true app-server --analytics-default-enabled"
        ));
        assert!(is_app_server_command(
            "/Applications/ChatGPT.app/Contents/Resources/codex --config=foo=bar -c baz=qux app-server"
        ));
        assert!(!is_app_server_command(
            "/Applications/ChatGPT.app/Contents/Resources/codex login status"
        ));
        assert!(!is_app_server_command(
            "/Applications/ChatGPT.app/Contents/Resources/codex --config app-server"
        ));
        assert!(!is_app_server_command(
            "/Applications/ChatGPT.app/Contents/Resources/codex exec app-server"
        ));
        assert!(!is_app_server_command(
            "/Applications/ChatGPT.app/Contents/Resources/codex --unknown app-server"
        ));
        assert!(!is_chatgpt_command(
            "/Applications/ChatGPT.app/Contents/Resources/native/bare-modifier-monitor"
        ));
    }
}
