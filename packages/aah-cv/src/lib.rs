//! GPU-accelerated template matching.
//!
//! Faster alternative to [imageproc::template_matching](https://docs.rs/imageproc/latest/imageproc/template_matching/index.html).

#![deny(clippy::all)]
// #![allow(dead_code)]
// #![allow(unused_variables)]

pub mod convolve;
pub mod fft;
pub mod gpu;
pub mod matching;
pub mod template_matching;
pub mod utils;

use image::{ImageBuffer, Luma};
use imageproc::template_matching::Extremes;
use std::{
    borrow::Cow,
    mem::size_of,
    ops::{Add, Div, Mul, Sub},
};
use template_matching::square_sum_arr2;
use utils::{image_mean, square_sum};
use wgpu::util::DeviceExt;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MatchTemplateMethod {
    SumOfAbsoluteErrors,
    SumOfSquaredErrors,
    CrossCorrelation,
    CCOEFF,
    CCOEFF_NORMED,
}

/// Slides a template over the input and scores the match at each point using the requested method.
///
/// This is a shorthand for:
/// ```ignore
/// let mut matcher = TemplateMatcher::new();
/// matcher.match_template(input, template, method);
/// matcher.wait_for_result().unwrap()
/// ```
/// You can use  [find_extremes] to find minimum and maximum values, and their locations in the result image.
pub fn match_template<'a>(
    input: &ImageBuffer<Luma<f32>, Vec<f32>>,
    template: &ImageBuffer<Luma<f32>, Vec<f32>>,
    method: MatchTemplateMethod,
) -> Image<'static> {
    match method {
        MatchTemplateMethod::CCOEFF => ccoeff(input, template, false),
        MatchTemplateMethod::CCOEFF_NORMED => ccoeff(input, template, true),
        _ => {
            let mut matcher = TemplateMatcher::new();
            matcher.match_template(input.into(), template.into(), MatchTemplateMethod::CCOEFF);
            matcher.wait_for_result().unwrap()
        }
    }
}

pub fn ccoeff<'a>(
    input: &ImageBuffer<Luma<f32>, Vec<f32>>,
    template: &ImageBuffer<Luma<f32>, Vec<f32>>,
    normed: bool,
) -> Image<'static> {
    let mask = ImageBuffer::from_pixel(template.width(), template.height(), Luma([0.0f32]));

    // let mut ic = input.clone();
    let n = (template.width() * template.height()) as f32;
    let i: Image = input.into();
    let ic = i - ccorr(input, &mask) / n;

    let mut tc = template.clone();
    let mean_t = image_mean(&template);
    tc.enumerate_pixels_mut().for_each(|(_, _, pixel)| {
        pixel[0] -= mean_t;
    });

    // CCorr(I, T'*M)
    let ccorr_i_tc = ccorr(input, &tc);
    // CCorr(I, M)
    let ccorr_i_m = ccorr(input, &mask);
    let mean_tc = image_mean(&tc);

    // CCorr(I', T') = CCorr(I, T'*M) - sum(T'*M)/sum(M)*CCorr(I, M)
    let mut res = ccorr_i_tc + ccorr_i_m.clone() * (-mean_tc);

    if normed {
        let tc_sq_sum = square_sum(&tc);

        res
    } else {
        res
    }
}

pub fn ccorr<'a>(input: impl Into<Image<'a>>, template: impl Into<Image<'a>>) -> Image<'static> {
    let mut matcher = TemplateMatcher::new();
    matcher.match_template(input.into(), template.into(), MatchTemplateMethod::CrossCorrelation);
    matcher.wait_for_result().unwrap()
}

pub struct Match {
    pub location: (u32, u32),
    pub value: f32,
}

pub fn find_matches(
    input: &Image<'_>,
    template_width: u32,
    template_height: u32,
    threshold: f32,
) -> Vec<Match> {
    let mut matches: Vec<Match> = Vec::new();

    let input_width = input.width;
    let input_height = input.height;

    for y in 0..input_height {
        for x in 0..input_width {
            let idx = (y * input.width) + x;
            let value = input.data[idx as usize];

            if value < threshold {
                if let Some(m) = matches.iter_mut().rev().find(|m| {
                    ((m.location.0 as i32 - x as i32).abs() as u32) < template_width
                        && ((m.location.1 as i32 - y as i32).abs() as u32) < template_height
                }) {
                    if value > m.value {
                        m.location = (x, y);
                        m.value = value;
                    }
                    continue;
                } else {
                    matches.push(Match {
                        location: (x, y),
                        value,
                    });
                }
            }
        }
    }

    matches
}

/// Finds the smallest and largest values and their locations in an image.
pub fn find_extremes(input: &Image<'_>) -> Extremes<f32> {
    let mut min_value = f32::MAX;
    let mut min_value_location = (0, 0);
    let mut max_value = f32::MIN;
    let mut max_value_location = (0, 0);

    for y in 0..input.height {
        for x in 0..input.width {
            let idx = (y * input.width) + x;
            let value = input.data[idx as usize];

            if value < min_value {
                min_value = value;
                min_value_location = (x, y);
            }

            if value > max_value {
                max_value = value;
                max_value_location = (x, y);
            }
        }
    }

    Extremes {
        min_value,
        max_value,
        min_value_location,
        max_value_location,
    }
}

#[derive(Clone)]
pub struct Image<'a> {
    pub data: Cow<'a, [f32]>,
    pub width: u32,
    pub height: u32,
}

impl<'a> Image<'a> {
    fn sqrt(&self) -> Image<'a> {
        Image {
            data: self.data.iter().map(|v| v.sqrt()).collect::<Vec<f32>>().into(),
            width: self.width,
            height: self.height,
        }
    }
}

impl Mul<Image<'_>> for Image<'_> {
    type Output = Image<'static>;

    fn mul(self, rhs: Image<'_>) -> Self::Output {
        let mut data = Vec::with_capacity(self.data.len());
        for (a, b) in self.data.iter().zip(rhs.data.iter()) {
            data.push(a * b);
        }

        Image {
            data: Cow::Owned(data),
            width: self.width,
            height: self.height,
        }
    }

}

impl Mul<Image<'_>> for f32 {
    type Output = Image<'static>;

    fn mul(self, rhs: Image<'_>) -> Self::Output {
        rhs * self
    }
}

impl Mul<f32> for Image<'_> {
    type Output = Image<'static>;

    fn mul(self, rhs: f32) -> Self::Output {
        let data = self
            .data
            .iter()
            .map(|v| v * rhs)
            .collect::<Vec<f32>>()
            .into();

        Image {
            data,
            width: self.width,
            height: self.height,
        }
    }
}

impl Div<f32> for Image<'_> {
    type Output = Image<'static>;

    fn div(self, rhs: f32) -> Self::Output {
        self * (1.0 / rhs)
    }
}

impl<'a> Add for Image<'a> {
    type Output = Image<'a>;

    fn add(self, other: Image<'a>) -> Self::Output {
        let mut data = Vec::with_capacity(self.data.len());
        for (a, b) in self.data.iter().zip(other.data.iter()) {
            data.push(a + b);
        }

        Image {
            data: Cow::Owned(data),
            width: self.width,
            height: self.height,
        }
    }
}

impl<'a> Sub for Image<'a> {
    type Output = Image<'a>;

    fn sub(self, other: Image<'a>) -> Self::Output {
        let mut data = Vec::with_capacity(self.data.len());
        for (a, b) in self.data.iter().zip(other.data.iter()) {
            data.push(a - b);
        }

        Image {
            data: Cow::Owned(data),
            width: self.width,
            height: self.height,
        }
    }
}

impl<'a> Image<'a> {
    pub fn new(data: impl Into<Cow<'a, [f32]>>, width: u32, height: u32) -> Self {
        Self {
            data: data.into(),
            width,
            height,
        }
    }
}

impl<'a> From<&'a image::ImageBuffer<image::Luma<f32>, Vec<f32>>> for Image<'a> {
    fn from(img: &'a image::ImageBuffer<image::Luma<f32>, Vec<f32>>) -> Self {
        Self {
            data: Cow::Borrowed(img),
            width: img.width(),
            height: img.height(),
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderUniforms {
    input_width: u32,
    input_height: u32,
    template_width: u32,
    template_height: u32,
}

pub struct TemplateMatcher {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shader: wgpu::ShaderModule,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,

    last_pipeline: Option<wgpu::ComputePipeline>,
    last_method: Option<MatchTemplateMethod>,

    last_input_size: (u32, u32),
    last_template_size: (u32, u32),
    last_result_size: (u32, u32),

    uniform_buffer: wgpu::Buffer,
    input_buffer: Option<wgpu::Buffer>,
    template_buffer: Option<wgpu::Buffer>,
    result_buffer: Option<wgpu::Buffer>,
    staging_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,

    matching_ongoing: bool,
}

impl Default for TemplateMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateMatcher {
    pub fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = pollster::block_on(async {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
                .expect("Adapter request failed")
        });

        let (device, queue) = pollster::block_on(async {
            adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        label: None,
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits::default(),
                    },
                    None,
                )
                .await
                .expect("Device request failed")
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/matching.wgsl"));

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniform_buffer"),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            size: size_of::<ShaderUniforms>() as _,
            mapped_at_creation: false,
        });

        Self {
            instance,
            adapter,
            device,
            queue,
            shader,
            pipeline_layout,
            bind_group_layout,
            last_pipeline: None,
            last_method: None,
            last_input_size: (0, 0),
            last_template_size: (0, 0),
            last_result_size: (0, 0),
            uniform_buffer,
            input_buffer: None,
            template_buffer: None,
            result_buffer: None,
            staging_buffer: None,
            bind_group: None,
            matching_ongoing: false,
        }
    }

    /// Waits for the latest [match_template] execution and returns the result.
    /// Returns [None] if no matching was started.
    pub fn wait_for_result(&mut self) -> Option<Image<'static>> {
        if !self.matching_ongoing {
            return None;
        }
        self.matching_ongoing = false;

        let (result_width, result_height) = self.last_result_size;

        let buffer_slice = self.staging_buffer.as_ref().unwrap().slice(..);
        let (sender, receiver) = futures_intrusive::channel::shared::oneshot_channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |v| sender.send(v).unwrap());

        self.device.poll(wgpu::Maintain::Wait);

        pollster::block_on(async {
            let result;

            if let Some(Ok(())) = receiver.receive().await {
                let data = buffer_slice.get_mapped_range();
                result = bytemuck::cast_slice(&data).to_vec();
                drop(data);
                self.staging_buffer.as_ref().unwrap().unmap();
            } else {
                result = vec![0.0; (result_width * result_height) as usize]
            };

            Some(Image::new(result, result_width as _, result_height as _))
        })
    }

    /// Slides a template over the input and scores the match at each point using the requested method.
    /// To get the result of the matching, call [wait_for_result].
    pub fn match_template<'a>(
        &mut self,
        input: Image<'a>,
        template: Image<'a>,
        method: MatchTemplateMethod,
    ) {
        if self.matching_ongoing {
            // Discard previous result if not collected.
            self.wait_for_result();
        }

        if self.last_pipeline.is_none() || self.last_method != Some(method) {
            self.last_method = Some(method);

            let entry_point = match method {
                MatchTemplateMethod::SumOfAbsoluteErrors => "main_sae",
                MatchTemplateMethod::SumOfSquaredErrors => "main_sse",
                MatchTemplateMethod::CrossCorrelation => "main_cc",
                _ => panic!("not implemented yet"),
            };

            self.last_pipeline = Some(self.device.create_compute_pipeline(
                &wgpu::ComputePipelineDescriptor {
                    label: None,
                    layout: Some(&self.pipeline_layout),
                    module: &self.shader,
                    entry_point,
                },
            ));
        }

        let mut buffers_changed = false;

        let input_size = (input.width, input.height);
        if self.input_buffer.is_none() || self.last_input_size != input_size {
            buffers_changed = true;

            self.last_input_size = input_size;

            self.input_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("input_buffer"),
                    contents: bytemuck::cast_slice(&input.data),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                },
            ));
        } else {
            self.queue.write_buffer(
                self.input_buffer.as_ref().unwrap(),
                0,
                bytemuck::cast_slice(&input.data),
            );
        }

        let template_size = (template.width, template.height);
        if self.template_buffer.is_none() || self.last_template_size != template_size {
            self.queue.write_buffer(
                &self.uniform_buffer,
                0,
                bytemuck::cast_slice(&[ShaderUniforms {
                    input_width: input.width,
                    input_height: input.height,
                    template_width: template.width,
                    template_height: template.height,
                }]),
            );
            buffers_changed = true;

            self.last_template_size = template_size;

            self.template_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("template_buffer"),
                    contents: bytemuck::cast_slice(&template.data),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                },
            ));
        } else {
            self.queue.write_buffer(
                self.template_buffer.as_ref().unwrap(),
                0,
                bytemuck::cast_slice(&template.data),
            );
        }

        let result_width = input.width - template.width + 1;
        let result_height = input.height - template.height + 1;
        let result_buf_size = (result_width * result_height) as u64 * size_of::<f32>() as u64;

        if buffers_changed {
            self.last_result_size = (result_width, result_height);

            self.result_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("result_buffer"),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                size: result_buf_size,
                mapped_at_creation: false,
            }));

            self.staging_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("staging_buffer"),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                size: result_buf_size,
                mapped_at_creation: false,
            }));

            self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.input_buffer.as_ref().unwrap().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.template_buffer.as_ref().unwrap().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.result_buffer.as_ref().unwrap().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.uniform_buffer.as_entire_binding(),
                    },
                ],
            }));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("compute_pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(self.last_pipeline.as_ref().unwrap());
            compute_pass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
            compute_pass.dispatch_workgroups(
                (result_width as f32 / 16.0).ceil() as u32,
                (result_height as f32 / 16.0).ceil() as u32,
                1,
            );
        }

        encoder.copy_buffer_to_buffer(
            self.result_buffer.as_ref().unwrap(),
            0,
            self.staging_buffer.as_ref().unwrap(),
            0,
            result_buf_size,
        );

        self.queue.submit(std::iter::once(encoder.finish()));
        self.matching_ongoing = true;
    }
}
