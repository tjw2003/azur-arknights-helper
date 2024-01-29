use std::time::Duration;

use super::AdbCommand;

#[cfg(test)]
mod test {
    use crate::adb::host;

    use super::{ScreenCap, ShellCommand};

    #[test]
    fn test_screencap() {
        let mut host = host::connect_default().unwrap();
        let res = host
            .execute_local_command("127.0.0.1:16384".to_string(), ScreenCap)
            .unwrap();
        println!("{}", res.len())
    }

    #[test]
    fn test_minitouch() {
        let mut host = host::connect_default().unwrap();
        let res = host
            .execute_local_command(
                "127.0.0.1:16384".to_string(),
                ShellCommand::new("/data/local/tmp/minitouch -h".to_string()),
            )
            .unwrap();
        println!("{res}")
    }
}

/// shell:command
///
/// command is something like "cmd arg1 arg2 ..."
pub struct ShellCommand {
    command: String,
}

impl ShellCommand {
    pub fn new(command: String) -> Self {
        Self { command }
    }
}

impl AdbCommand for ShellCommand {
    type Output = String;

    fn raw_command(&self) -> String {
        format!("shell:{}", self.command)
    }

    fn handle_response(&self, host: &mut crate::adb::host::Host) -> Result<Self::Output, String> {
        host.check_response_status()?;
        let res = host.read_to_end_string()?;
        Ok(res)
    }
}

/// shell:screencap -p
pub struct ScreenCap;

impl ScreenCap {
    pub fn new() -> Self {
        Self
    }
}

impl AdbCommand for ScreenCap {
    type Output = Vec<u8>;

    fn raw_command(&self) -> String {
        "shell:screencap -p".to_string()
    }

    fn handle_response(&self, host: &mut crate::adb::host::Host) -> Result<Self::Output, String> {
        host.check_response_status()?;
        host.read_to_end()
    }
}

/// shell:input swipe x1 y1 x2 y2
pub struct InputSwipe {
    p1: (u32, u32),
    p2: (i32, i32),
    duration: Duration,
}

impl InputSwipe {
    pub fn new(p1: (u32, u32), p2: (i32, i32), duration: Duration) -> Self {
        Self { p1, p2, duration }
    }
}

impl AdbCommand for InputSwipe {
    type Output = ();

    fn raw_command(&self) -> String {
        format!(
            "shell:input swipe {} {} {} {} {}",
            self.p1.0,
            self.p1.1,
            self.p2.0,
            self.p2.1,
            self.duration.as_millis()
        )
    }

    fn handle_response(&self, host: &mut crate::adb::host::Host) -> Result<Self::Output, String> {
        host.check_response_status()
    }
}
