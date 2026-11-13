use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub ok: bool,
    pub output: String,
}
#[derive(Clone)]
pub struct NginxProcess {
    bin: PathBuf,
    timeout: Duration,
}
impl NginxProcess {
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self {
            bin: bin.into(),
            timeout: Duration::from_secs(30),
        }
    }
    #[cfg(test)]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    async fn run(&self, args: &[&str]) -> Result<(i32, String)> {
        let mut child = Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let operation = async {
            let mut output = Vec::new();
            let mut out_buf = [0; 4096];
            let mut err_buf = [0; 4096];
            let (mut out_open, mut err_open) = (true, true);
            while out_open || err_open {
                tokio::select! {
                    n=stdout.read(&mut out_buf),if out_open => {let n=n?;if n==0 {out_open=false;}else {output.extend_from_slice(&out_buf[..n]);}},
                    n=stderr.read(&mut err_buf),if err_open => {let n=n?;if n==0 {err_open=false;}else {output.extend_from_slice(&err_buf[..n]);}},
                }
            }
            let status = child.wait().await?;
            Ok::<_, anyhow::Error>((
                status.code().unwrap_or(1),
                String::from_utf8_lossy(&output).into_owned(),
            ))
        };
        match tokio::time::timeout(self.timeout, operation).await {
            Ok(result) => result,
            Err(_) => {
                let _ = child.kill().await;
                bail!("nginx command timed out");
            }
        }
    }
    pub async fn validate(&self) -> ValidationResult {
        match self.run(&["-t"]).await {
            Ok((code, output)) => ValidationResult {
                ok: code == 0,
                output,
            },
            Err(e) => ValidationResult {
                ok: false,
                output: e.to_string(),
            },
        }
    }
    #[cfg(test)]
    pub async fn reload(&self) -> Result<()> {
        let (code, output) = self.run(&["-s", "reload"]).await?;
        if code != 0 {
            bail!("nginx -s reload failed (exit code {code}): {output}");
        }
        Ok(())
    }

}

#[cfg(test)]
#[path="g13_process_kernel_tests.rs"]
mod g13_kernel;
