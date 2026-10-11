//! Cooperative export cancellation shared by desktop worker and backend.
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
#[derive(Clone, Default)]
pub struct ExportControl {
    cancelled: Arc<AtomicBool>,
    log: Option<Arc<dyn Fn(String) + Send + Sync>>,
    phase_sink: Option<Arc<dyn Fn(crate::export_progress::ExportPhase) + Send + Sync>>,
    logged_once: Arc<Mutex<BTreeMap<String, String>>>,
}
#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Export cancelled")
    }
}
impl std::error::Error for Cancelled {}
impl ExportControl {
    pub fn with_log(log: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            log: Some(Arc::new(log)),
            ..Self::default()
        }
    }
    pub fn with_phase_sink(
        mut self,
        sink: impl Fn(crate::export_progress::ExportPhase) + Send + Sync + 'static,
    ) -> Self {
        self.phase_sink = Some(Arc::new(sink));
        self
    }
    pub fn report_phase(&self, phase: crate::export_progress::ExportPhase) {
        if let Some(sink) = &self.phase_sink {
            sink(phase);
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn check(&self) -> anyhow::Result<()> {
        if self.is_cancelled() {
            Err(Cancelled.into())
        } else {
            Ok(())
        }
    }
    pub fn log(&self, text: impl Into<String>) {
        if let Some(sink) = &self.log {
            sink(text.into());
        }
    }
    /// Emit a keyed warning at most once for this shared render control. The
    /// stored message is also available to the structured diagnostic report.
    pub fn log_once(&self, key: impl Into<String>, text: impl Into<String>) -> bool {
        let key = key.into();
        let text = text.into();
        let should_log = match self.logged_once.lock() {
            Ok(mut logged) if !logged.contains_key(&key) => {
                logged.insert(key, text.clone());
                true
            }
            Ok(_) => false,
            Err(_) => true,
        };
        if should_log {
            self.log(text);
        }
        should_log
    }
    pub fn logged_once_messages(&self) -> Vec<String> {
        self.logged_once
            .lock()
            .map(|messages| messages.values().cloned().collect())
            .unwrap_or_default()
    }
    /// Captured files avoid pipe deadlocks; every owned child is reaped on cancel.
    pub(crate) fn output(&self, command: &mut Command) -> anyhow::Result<Output> {
        self.check()?;
        hide_child_window(command);
        let mut stdout = tempfile::tempfile()?;
        let mut stderr = tempfile::tempfile()?;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout.try_clone()?))
            .stderr(Stdio::from(stderr.try_clone()?))
            .spawn()?;
        let status = loop {
            if self.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Cancelled.into());
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.into());
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        stdout.seek(SeekFrom::Start(0))?;
        stderr.seek(SeekFrom::Start(0))?;
        let mut out = Vec::new();
        let mut err = Vec::new();
        stdout.read_to_end(&mut out)?;
        stderr.read_to_end(&mut err)?;
        self.check()?;
        Ok(Output {
            status,
            stdout: out,
            stderr: err,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_reaps_a_running_media_child() {
        let Some(tools) = crate::export_mv::tests::tools() else {
            return;
        };
        let control = ExportControl::default();
        let cancel = control.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            cancel.cancel();
        });
        let started = std::time::Instant::now();
        let result = control.output(Command::new(&tools.ffmpeg).args([
            "-v", "error", "-re", "-f", "lavfi", "-i", "anullsrc", "-t", "60", "-f", "null", "-",
        ]));
        worker.join().unwrap();
        assert!(result.unwrap_err().is::<Cancelled>());
        assert!(started.elapsed().as_secs_f64() < 3.0);
    }

    #[test]
    fn token_is_shared_and_error_is_typed() {
        let a = ExportControl::default();
        let b = a.clone();
        assert!(b.check().is_ok());
        a.cancel();
        assert!(b.check().unwrap_err().is::<Cancelled>());
    }

    #[test]
    fn keyed_warnings_are_logged_once_and_remain_available_to_diagnostics() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = output.clone();
        let control = ExportControl::with_log(move |message| sink.lock().unwrap().push(message));
        assert!(control.log_once("family", "warning text"));
        assert!(!control.clone().log_once("family", "warning text"));
        assert_eq!(*output.lock().unwrap(), ["warning text"]);
        assert_eq!(control.logged_once_messages(), ["warning text"]);
    }
}

/// Owned media tools must never open console windows over the desktop UI.
pub(crate) fn hide_child_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}
