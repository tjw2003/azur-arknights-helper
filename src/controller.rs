use std::{
    error::Error,
    fs,
    io::Cursor,
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use image::{ImageBuffer, math::Rect};

use crate::{adb::{connect, Device, MyError}, config::task::TaskConfig, task::{Exec, Task}};

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_controller() {
        let mut controller =
            Controller::connect("127.0.0.1:16384").expect("failed to connect to device");
        controller.screencap();
    }

    #[test]
    fn test_exec_task() -> Result<(), Box<dyn Error>> {
        let controller =
            Controller::connect("127.0.0.1:16384").expect("failed to connect to device");
        controller.exec_task("award")?;
        Ok(())
    }
}

pub struct Controller {
    pub inner: Device,
    width: u32,
    height: u32,
}

impl Controller {
    pub fn connect<S: AsRef<str>>(device_serial: S) -> Result<Self, MyError> {
        let device = connect(device_serial)?;
        let controller = Self {
            inner: device,
            width: 0,
            height: 0,
        };
        let screen = controller.screencap()?;

        let controller = Self {
            width: screen.width(),
            height: screen.width(),
            ..controller
        };
        Ok(controller)
    }
}

impl Controller {
    pub fn exec_task<S: AsRef<str>>(&self, name: S) -> Result<(), Box<dyn Error>> {
        let name = name.as_ref().to_string();
        let task_config = TaskConfig::load()?;
        let task = task_config.0.get(&name).ok_or("failed to get task")?.clone();
        println!("executing {:?}", task);
        task.run(self)?;
        Ok(())
    }

    pub fn click_in_rect(&self, rect: Rect) -> Result<(), MyError> {
        let x = rand::random::<u32>() % rect.width + rect.x;
        let y = rand::random::<u32>() % rect.height + rect.y;
        self.click(x, y)
    }

    pub fn click(&self, x: u32, y: u32) -> Result<(), MyError> {
        println!("clicking {}, {}", x, y);
        self.inner
            .execute_command_by_process(format!("shell input tap {} {}", x, y).as_str())?;
        Ok(())
    }

    pub fn swipe(
        &self,
        start: (u32, u32),
        end: (u32, u32),
        duration: Duration,
    ) -> Result<(), MyError> {
        self.inner.execute_command_by_process(
            format!(
                "shell input swipe {} {} {} {} {}",
                start.0,
                start.1,
                end.0,
                end.1,
                duration.as_millis()
            )
            .as_str(),
        )?;
        Ok(())
    }

    pub fn screencap(&self) -> Result<image::DynamicImage, MyError> {
        self.inner.screencap()
    }

    pub fn press_home(&self) -> Result<(), MyError> {
        self.inner
            .execute_command_by_process("shell input keyevent HOME")?;
        Ok(())
    }

    pub fn press_esc(&self) -> Result<(), MyError> {
        self.inner
            .execute_command_by_process("shell input keyevent 111")?;
        Ok(())
    }
}
