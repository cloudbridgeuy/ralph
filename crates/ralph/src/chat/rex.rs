use std::process::Command;

pub(super) fn split(argv: &[String]) -> Result<(), String> {
    match Command::new("rex").args(argv).output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "rex split failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(e) => Err(format!("could not run rex: {e}")),
    }
}
