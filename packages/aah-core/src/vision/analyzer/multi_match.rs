use color_print::{cformat, cprintln};
use image::DynamicImage;

use crate::{
    controller::DEFAULT_HEIGHT,
    vision::{
        matcher::multi_matcher::{MultiMatcher, MultiMatcherResult},
        utils::{binarize_image, draw_box, Rect},
    },
    AAH,
};

use super::Analyzer;

pub struct MultiMatchAnalyzerOutput {
    pub screen: Box<DynamicImage>,
    pub res: MultiMatcherResult,
    pub annotated_screen: Box<DynamicImage>,
}

pub struct MultiMatchAnalyzer {
    template_filename: String,
    binarize_threshold: Option<u8>,
    threshold: Option<f32>,
    use_cache: bool,
    roi: [(f32, f32); 2], // topleft and bottomright
}

impl MultiMatchAnalyzer {
    pub fn new<S: AsRef<str>>(
        template_filename: S,
        binarize_threshold: Option<u8>,
        threshold: Option<f32>,
    ) -> Self {
        let template_filename = template_filename.as_ref().to_string();
        Self {
            template_filename,
            binarize_threshold,
            threshold,
            use_cache: false,
            roi: [(0.0, 0.0), (1.0, 1.0)], // topleft and bottomright
        }
    }

    pub fn use_cache(mut self) -> Self {
        self.use_cache = true;
        self
    }

    pub fn roi(mut self, tl: (f32, f32), br: (f32, f32)) -> Self {
        self.roi = [tl, br];
        self
    }
}

impl Analyzer for MultiMatchAnalyzer {
    type Output = MultiMatchAnalyzerOutput;
    fn analyze(&mut self, core: &AAH) -> Result<Self::Output, String> {
        // Make sure that we are in the operation-start page
        let log_tag = cformat!("<strong>[MultiMatchAnalyzer]: </strong>");
        cprintln!(
            "{log_tag}matching {:?}",
            self.template_filename
        );

        // TODO: 并不是一个好主意，缩放大图消耗时间更多，且误差更大
        // TODO: 然而测试了一下，发现缩放模板有时也会导致误差较大 (333.9063)
        // let image = aah
        //     .controller
        //     .screencap_scaled()
        //     .map_err(|err| format!("{:?}", err))?;

        // Get Screen
        let screen = if self.use_cache {
            core.screen_cache_or_cap()?.clone()
        } else {
            core.screen_cap_and_cache()
                .map_err(|err| format!("{:?}", err))?
        };
        let tl = (
            self.roi[0].0 * screen.width() as f32,
            self.roi[0].1 * screen.height() as f32,
        );
        let br = (
            self.roi[1].0 * screen.width() as f32,
            self.roi[1].1 * screen.height() as f32,
        );
        let tl = (tl.0 as u32, tl.1 as u32);
        let br = (br.0 as u32, br.1 as u32);

        let cropped = screen.crop_imm(tl.0, tl.1, br.0 - tl.0, br.1 - tl.1);
        let template = core.get_template(&self.template_filename).unwrap();

        // Scaling
        let template = if screen.height() != DEFAULT_HEIGHT {
            let scale_factor = screen.height() as f32 / DEFAULT_HEIGHT as f32;

            let new_width = (template.width() as f32 * scale_factor) as u32;
            let new_height = (template.height() as f32 * scale_factor) as u32;

            DynamicImage::ImageRgba8(image::imageops::resize(
                &template,
                new_width,
                new_height,
                image::imageops::FilterType::Lanczos3,
            ))
        } else {
            template
        };

        // Binarize
        let (image, template) = match self.binarize_threshold {
            Some(threshold) => (
                binarize_image(&screen, threshold),
                binarize_image(&template, threshold),
            ),
            None => (screen.clone(), template),
        };

        // Match
        let res = MultiMatcher::Template {
            image: cropped.to_luma32f(), // use cropped
            template: template.to_luma32f(),
            threshold: self.threshold,
        }
        .result();
        let res = MultiMatcherResult {
            rects: res
                .rects
                .into_iter()
                .map(|rect| Rect {
                    x: rect.x + tl.0,
                    y: rect.y + tl.1,
                    ..rect
                })
                .collect(),
            ..res
        };

        // Annotate
        let mut annotated_screen = screen.clone();
        for rect in &res.rects {
            draw_box(
                &mut annotated_screen,
                rect.x as i32,
                rect.y as i32,
                rect.width,
                rect.height,
                [255, 0, 0, 255],
            );
        }

        let screen = Box::new(screen);
        let annotated_screen = Box::new(annotated_screen);
        Ok(Self::Output {
            screen,
            res,
            annotated_screen,
        })
    }
}

#[cfg(test)]
mod test {
    use crate::{
        vision::analyzer::{multi_match::MultiMatchAnalyzer, Analyzer},
        AAH,
    };

    #[test]
    fn test_multi_template_match_analyzer() {
        let mut core = AAH::connect("127.0.0.1:16384", "../../resources", |_| {}).unwrap();
        let mut analyzer =
            MultiMatchAnalyzer::new("battle_deploy-card-cost0".to_string(), Some(127), None);
        let output = analyzer.analyze(&mut core).unwrap();
        println!("{:?}", output.res.rects);
    }
}
