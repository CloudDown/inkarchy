//! Secret Service via its standard Linux helper. Keys go through stdin, not argv.
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub enum CredentialAction {
    Load,
    // Keep keyring operations available even while the compact AI panel only loads keys.
    #[allow(dead_code)]
    Save(String),
    #[allow(dead_code)]
    Delete,
}
pub struct CredentialJob {
    rx: Receiver<Result<String, String>>,
}

impl CredentialJob {
    pub fn start(action: CredentialAction, ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new()
            .name("inkarchy-keyring".into())
            .spawn(move || {
                let result = run(action);
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        Self { rx }
    }
    pub fn poll(&self) -> Option<Result<String, String>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err(
                "Keyring unavailable. Use a key for this session.".into()
            )),
        }
    }
}

fn run(action: CredentialAction) -> Result<String, String> {
    let mut command = Command::new("secret-tool");
    match &action {
        CredentialAction::Load => {
            command.arg("lookup");
        }
        CredentialAction::Save(_) => {
            command.args(["store", "--label=Inkarchy Gemini API key"]);
        }
        CredentialAction::Delete => {
            command.arg("clear");
        }
    }
    command
        .args(["application", "inkarchy", "service", "gemini"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| "System keyring unavailable. Use a key for this session.".to_owned())?;
    if let Some(mut input) = child.stdin.take() {
        if let CredentialAction::Save(key) = &action {
            if key.len() > 4096 || input.write_all(key.trim().as_bytes()).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Could not save the API key.".into());
            }
        }
    }
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err("Keyring entry unavailable. Enter a key to use for this session or save again.".into());
                }
                let mut output = String::new();
                if let Some(stdout) = child.stdout.take() {
                    stdout
                        .take(4097)
                        .read_to_string(&mut output)
                        .map_err(|_| "Could not read the keyring entry.".to_owned())?;
                }
                if output.len() > 4096 {
                    return Err("Invalid keyring entry.".into());
                }
                return Ok(output.trim().to_owned());
            }
            Ok(None) if start.elapsed() < Duration::from_secs(30) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Keyring timed out. Use a key for this session.".into());
            }
        }
    }
}
