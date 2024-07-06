#![feature(associated_type_defaults)]
#![feature(path_file_prefix)]

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use config::{navigate::NavigateConfig, task::TaskConfig};
use controller::{aah_controller::AahController, Controller};
use ocrs::{OcrEngine, OcrEngineParams};
use rten::Model;
use task::TaskEvt;
use vision::analyzer::{
    battle::{
        deploy::{DeployAnalyzer, DeployAnalyzerOutput},
        BattleAnalyzer, BattleState,
    },
    Analyzer,
};

use crate::task::Task;

pub mod adb;
pub mod config;
pub mod controller;
pub mod task;
pub mod vision;

/// AAH 的实例
pub struct AAH {
    pub res_dir: PathBuf,
    /// [`controller`] 承担设备控制相关操作（比如触摸、截图等）
    pub controller: Box<dyn Controller + Sync + Send>,
    /// 由 `tasks.toml` 和 `tasks` 目录加载的任务配置
    pub task_config: TaskConfig,
    /// 由 `navigates.toml` 加载的导航配置
    pub navigate_config: NavigateConfig,
    // /// 屏幕内容的缓存
    screen_cache: Mutex<Option<image::DynamicImage>>,
    on_task_evt: Box<dyn Fn(TaskEvt) + Sync + Send>,
    ocr_engine: OcrEngine,
}

pub fn init_ocr_engine<P: AsRef<Path>>(res_dir: P) -> OcrEngine {
    let res_dir = res_dir.as_ref();
    let detection_model_path = res_dir
        .join("models")
        .join("ocrs")
        .join("text-detection.rten");
    let rec_model_path = res_dir
        .join("models")
        .join("ocrs")
        .join("text-recognition.rten");

    let detection_model = Model::load_file(detection_model_path).unwrap();
    let recognition_model = Model::load_file(rec_model_path).unwrap();

    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })
    .unwrap();
    engine
}

impl AAH {
    /// 连接到 `serial` 指定的设备（`serial` 就是 `adb devices` 里的序列号）
    /// - `serial`: 设备的序列号
    /// - `res_dir`: 资源目录的路径
    /// - `on_task_evt`: 任务事件的回调函数
    pub fn connect<S: AsRef<str>, P: AsRef<Path>, F: Fn(TaskEvt) + Send + Sync + 'static>(
        serial: S,
        res_dir: P,
        on_task_evt: F,
    ) -> Result<Self, Box<dyn Error>> {
        let res_dir = res_dir.as_ref().to_path_buf();
        let task_config =
            TaskConfig::load(&res_dir).map_err(|err| format!("task config not found: {err}"))?;
        let navigate_config = NavigateConfig::load(&res_dir)
            .map_err(|err| format!("navigate config not found: {err}"))?;
        // let controller = Box::new(AdbInputController::connect(serial)?);
        let controller = Box::new(AahController::connect(serial, &res_dir)?);

        let ocr_engine = init_ocr_engine(&res_dir);
        Ok(Self {
            res_dir,
            controller,
            task_config,
            navigate_config,
            on_task_evt: Box::new(on_task_evt),
            screen_cache: Mutex::new(None),
            ocr_engine,
        })
    }

    /// 运行名为 `name` 的任务
    ///
    /// - `name`: 任务名称
    pub fn run_task<S: AsRef<str>>(&self, name: S) -> Result<(), String> {
        let name = name.as_ref().to_string();

        let task = self
            .task_config
            .0
            .get(&name)
            .ok_or("failed to get task")?
            .clone();
        println!("executing {:?}", task);

        task.run(self)?;

        Ok(())
    }

    /// Get screen cache or capture one. This is for internal analyzer use
    fn screen_cache_or_cap(&self) -> Result<image::DynamicImage, String> {
        let mut screen_cache = self.screen_cache.lock().unwrap();
        if screen_cache.is_none() {
            let screen = self
                .controller
                .screencap()
                .map_err(|err| format!("{err}"))?;
            *screen_cache = Some(screen.clone());
        }
        screen_cache
            .as_ref()
            .map(|i| i.clone())
            .ok_or("screen cache is empty".to_string())
    }

    fn screen_cap_and_cache(&self) -> Result<image::DynamicImage, String> {
        let mut screen_cache = self.screen_cache.lock().unwrap();
        let screen = self
            .controller
            .screencap()
            .map_err(|err| format!("{err}"))?;
        *screen_cache = Some(screen);
        screen_cache
            .as_ref()
            .map(|i| i.clone())
            .ok_or("screen cache is empty".to_string())
    }

    /// Capture a screen, and return decoded image
    pub fn get_screen(&mut self) -> Result<image::DynamicImage, String> {
        self.controller.screencap().map_err(|err| format!("{err}"))
        // match &self.screen_cache {
        //     Some(cache) => Ok(cache.clone()),
        //     None => {
        //         self.update_screen()?;
        //         Ok(self.screen_cache.as_ref().unwrap().clone())
        //     }
        // }
    }

    /// Capture a screen, and return raw data in Png format
    pub fn get_raw_screen(&mut self) -> Result<Vec<u8>, String> {
        self.controller
            .raw_screencap()
            .map_err(|err| format!("{err}"))
    }

    /// 重新加载 resources 中的配置
    pub fn reload_resources(&mut self) -> Result<(), String> {
        let task_config = TaskConfig::load(&self.res_dir)
            .map_err(|err| format!("task config not found: {err}"))?;
        let navigate_config = NavigateConfig::load(&self.res_dir)
            .map_err(|err| format!("navigate config not found: {err}"))?;
        self.task_config = task_config;
        self.navigate_config = navigate_config;
        Ok(())
    }

    /// 从 `{res_path}/resources/templates/1920x1080` 目录中根据文件名称获取模板
    /// - `name` 为完整文件名
    pub fn get_template<S: AsRef<str>>(&self, name: S) -> Result<image::DynamicImage, String> {
        let name = name.as_ref();
        let path = self.res_dir.join("templates").join("1920x1080").join(name);
        let image = image::open(path).map_err(|err| format!("template not found: {err}"))?;
        Ok(image)
    }

    fn get_oper_avatars(&self, name: &str) -> Result<Vec<image::DynamicImage>, String> {
        let components = name.split("_").collect::<Vec<&str>>();
        let path = self
            .res_dir
            .join("avatars")
            .join("char")
            .join(components[1]);

        let mut images = vec![];
        for f in fs::read_dir(path).unwrap() {
            let f = f.unwrap();
            println!("{:?}", f.path());
            let image = image::open(f.path()).map_err(|err| format!("avatar not found: {err}"))?;
            // let image = image.crop_imm(30, 20, 100, 120);
            images.push(image);
        }
        Ok(images)
    }

    /// 截取当前帧的屏幕内容，分析部署卡片，返回 [`DeployAnalyzerOutput`]
    pub fn analyze_deploy(&self) -> Result<DeployAnalyzerOutput, String> {
        let mut analyzer = DeployAnalyzer::new();
        analyzer.analyze(self)
    }

    /// 获取所有任务名称
    pub fn get_tasks(&self) -> Vec<String> {
        self.task_config.0.keys().map(|s| s.to_string()).collect()
    }

    /// 发起事件
    pub(crate) fn emit_task_evt(&self, evt: TaskEvt) {
        (self.on_task_evt)(evt)
    }

    /// 启动战斗分析器，直到战斗结束
    ///
    /// 分析信息会通过 [`TaskEvt::BattleAnalyzerRes`] 事件返回，
    /// 出于性能考虑，目前待部署区只设置了识别以下几位干员：
    /// - `char_285_medic2`
    /// - `char_502_nblade`
    /// - `char_500_noirc`
    /// - `char_503_rang`
    /// - `char_501_durin`
    /// - `char_284_spot`
    /// - `char_212_ansel`
    /// - `char_208_melan`
    /// - `char_151_myrtle`
    pub fn start_battle_analyzer(&self) {
        let mut analyzer = BattleAnalyzer::new();
        while analyzer.battle_state != BattleState::Completed {
            let output = analyzer.analyze(self).unwrap();
            self.emit_task_evt(TaskEvt::BattleAnalyzerRes(output));
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        path::Path,
        sync::{Mutex, OnceLock},
        thread::sleep,
        time::Duration,
    };

    use super::*;

    #[test]
    fn foo() {
        let aah = AAH::connect("127.0.0.1:16384", "../../resources", |evt| {
            if let TaskEvt::BattleAnalyzerRes(res) = evt {
                println!("{:?}", res);
            }
        })
        .unwrap();
        aah.start_battle_analyzer()
    }

    #[test]
    fn test_get_tasks() {
        static s: OnceLock<Mutex<Option<AAH>>> = OnceLock::new();
        let aah = AAH::connect("127.0.0.1:16384", "../../resources", |_| {}).unwrap();
        println!("{:?}", aah.get_tasks());
    }

    fn save_screenshot<P: AsRef<Path>, S: AsRef<str>>(aah: &mut AAH, path: P, name: S) {
        let path = path.as_ref();
        let name = name.as_ref();

        let target_path = path.join(name);
        println!("saving screenshot to {:?}", target_path);

        // aah.update_screen().unwrap();
        let screen = aah.get_screen().unwrap();
        screen
            .save_with_format(target_path, image::ImageFormat::Png)
            .unwrap();
    }

    #[test]
    fn screenshot() {
        let mut aah = AAH::connect("127.0.0.1:16384", "../../resources", |_| {}).unwrap();
        let dir = "../../resources/templates/MUMU-1920x1080";
        // save_screenshot(dir, "start.png");
        // save_screenshot(dir, "wakeup.png");
        // save_screenshot(dir, "notice.png");
        // save_screenshot(dir, "main.png");
        // save_screenshot(dir, "confirm.png");
        save_screenshot(&mut aah, dir, "1-4_deploying.png");
        // let dir = "/Volumes/Data/Dev/AahAI/dataset/1-4/img";
        // for i in 0..10 {
        //     save_screenshot(&mut aah, dir, format!("{i}.png"));
        //     sleep(Duration::from_secs_f32(0.2))
        // }
        // let dir = "../aah-resource/assets";
        // save_screenshot(dir, "LS-6_1.png");
    }
}
