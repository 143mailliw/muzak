use std::{
    collections::VecDeque,
    ops::Range,
    sync::mpsc::{self, Receiver, Sender},
};

use crate::{
    devices::{
        errors::{
            CloseError, FindError, InfoError, InitializationError, ListError, OpenError,
            SubmissionError,
        },
        format::{BufferSize, ChannelSpec, FormatInfo, SampleFormat, SupportedFormat},
        traits::{Device, DeviceProvider, OutputStream},
    },
    media::playback::{GetInnerSamples, PlaybackFrame},
};
use cpal::{
    traits::{DeviceTrait, HostTrait},
    Host, SizedSample,
};
use rb::{RbConsumer, RbProducer, SpscRb, RB};

pub struct CpalProvider {
    host: Host,
}

impl Default for CpalProvider {
    fn default() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }
}

impl DeviceProvider for CpalProvider {
    fn initialize(&mut self) -> Result<(), InitializationError> {
        self.host = cpal::default_host();
        Ok(())
    }

    fn get_devices(&mut self) -> Result<Vec<impl Device>, ListError> {
        Ok(self
            .host
            .devices()
            .map_err(|_| ListError::Unknown)? // TODO: Requires platform-specific error handling
            .into_iter()
            .map(|dev| CpalDevice::from(dev))
            .collect())
    }

    fn get_default_device(&mut self) -> Result<impl Device, FindError> {
        self.host
            .default_output_device()
            .ok_or(FindError::DeviceDoesNotExist)
            .map(|dev| CpalDevice::from(dev))
    }

    fn get_device_by_uid(&mut self, id: &String) -> Result<impl Device, FindError> {
        self.host
            .devices()
            .map_err(|_| FindError::Unknown)? // TODO: Requires platform-specific error handling
            .into_iter()
            .find(|dev| dev.name().unwrap_or("NULL".into()) == *id)
            .ok_or(FindError::DeviceDoesNotExist)
            .map(|dev| CpalDevice::from(dev))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CpalEvent {
    NextSegment,
}

struct CpalDevice {
    device: cpal::Device,
}

impl From<cpal::Device> for CpalDevice {
    fn from(value: cpal::Device) -> Self {
        CpalDevice { device: value }
    }
}

fn format_from_cpal(format: &cpal::SampleFormat) -> SampleFormat {
    match format {
        cpal::SampleFormat::I8 => SampleFormat::Signed8,
        cpal::SampleFormat::I16 => SampleFormat::Signed16,
        cpal::SampleFormat::I32 => SampleFormat::Signed32,
        cpal::SampleFormat::U8 => SampleFormat::Unsigned8,
        cpal::SampleFormat::U16 => SampleFormat::Unsigned16,
        cpal::SampleFormat::U32 => SampleFormat::Unsigned32,
        cpal::SampleFormat::F32 => SampleFormat::Float32,
        cpal::SampleFormat::F64 => SampleFormat::Float64,
        _ => SampleFormat::Unsupported, // should never happen
    }
}

fn cpal_from_format(format: &SampleFormat) -> cpal::SampleFormat {
    match format {
        SampleFormat::Signed8 => cpal::SampleFormat::I8,
        SampleFormat::Signed16 => cpal::SampleFormat::I16,
        SampleFormat::Signed32 => cpal::SampleFormat::I32,
        SampleFormat::Unsigned8 => cpal::SampleFormat::U8,
        SampleFormat::Unsigned16 => cpal::SampleFormat::U16,
        SampleFormat::Unsigned32 => cpal::SampleFormat::U32,
        SampleFormat::Float32 => cpal::SampleFormat::F32,
        SampleFormat::Float64 => cpal::SampleFormat::F64,
        _ => cpal::SampleFormat::U16, // should be impossible
    }
}

fn cpal_config_from_info(format: &FormatInfo) -> Result<cpal::StreamConfig, ()> {
    if format.originating_provider != "cpal" {
        Err(())
    } else {
        Ok(cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(format.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        })
    }
}

fn interleave<T>(samples: Vec<Vec<T>>) -> Vec<T>
where
    T: Copy + PartialEq,
{
    if samples.is_empty() {
        return vec![];
    }

    let length = samples.len();
    let mut result = vec![];

    for i in 0..(samples.len() * samples[0].len()) {
        result.push(samples[i % length][i / length]);
    }

    result
}

impl CpalDevice {
    fn create_stream<T: SizedSample + GetInnerSamples + Default + Send + Sized + 'static>(
        &mut self,
        format: FormatInfo,
    ) -> Result<Box<dyn OutputStream>, OpenError> {
        let config =
            cpal_config_from_info(&format).map_err(|_| OpenError::InvalidConfigProvider)?;

        let channels = match format.channels {
            ChannelSpec::Count(v) => v,
            _ => panic!("non cpal device"),
        };

        let buffer_size = ((200 * config.sample_rate.0 as usize) / 1000) * channels as usize;
        let rb: SpscRb<T> = SpscRb::new(buffer_size);
        let cons = rb.consumer();
        let prod = rb.producer();

        let stream = self
            .device
            .build_output_stream(
                &config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    cons.read(data).unwrap_or(0);
                },
                move |err| {},
                None,
            )
            .map_err(|_| OpenError::Unknown)?;

        Ok(Box::new(CpalStream {
            ring_buf: prod,
            stream,
            format,
        }))
    }
}

impl Device for CpalDevice {
    fn open_device(&mut self, format: FormatInfo) -> Result<Box<dyn OutputStream>, OpenError> {
        if format.originating_provider != "cpal" {
            Err(OpenError::InvalidConfigProvider)
        } else {
            match format.sample_type {
                SampleFormat::Signed8 => self.create_stream::<i8>(format),
                SampleFormat::Signed16 => self.create_stream::<i16>(format),
                SampleFormat::Signed32 => self.create_stream::<i32>(format),
                SampleFormat::Unsigned8 => self.create_stream::<u8>(format),
                SampleFormat::Unsigned16 => self.create_stream::<u16>(format),
                SampleFormat::Unsigned32 => self.create_stream::<u32>(format),
                SampleFormat::Float32 => self.create_stream::<f32>(format),
                SampleFormat::Float64 => self.create_stream::<f64>(format),
                _ => Err(OpenError::InvalidSampleFormat),
            }
        }
    }

    fn get_supported_formats(&self) -> Result<Vec<SupportedFormat>, InfoError> {
        Ok(self
            .device
            .supported_output_configs()
            .map_err(|_| InfoError::Unknown)?
            .filter(|c| {
                let format = c.sample_format();
                format != cpal::SampleFormat::I64 && format != cpal::SampleFormat::U64
            })
            .map(|c| SupportedFormat {
                originating_provider: "cpal",
                sample_type: format_from_cpal(&c.sample_format()),
                sample_rates: Range {
                    start: c.min_sample_rate().0,
                    end: c.max_sample_rate().0,
                },
                buffer_size: match c.buffer_size() {
                    cpal::SupportedBufferSize::Range { min, max } => BufferSize::Range(Range {
                        start: *min,
                        end: *max,
                    }),
                    cpal::SupportedBufferSize::Unknown => BufferSize::Unknown,
                },
                channels: ChannelSpec::Count(c.channels()),
            })
            .collect())
    }

    fn get_default_format(&self) -> Result<FormatInfo, InfoError> {
        let format = self
            .device
            .default_output_config()
            .map_err(|_| InfoError::Unknown)?;
        Ok(FormatInfo {
            originating_provider: "cpal",
            sample_type: format_from_cpal(&format.sample_format()),
            sample_rate: format.sample_rate().0,
            buffer_size: match format.buffer_size() {
                cpal::SupportedBufferSize::Range { min, max } => BufferSize::Range(Range {
                    start: *min,
                    end: *max,
                }),
                cpal::SupportedBufferSize::Unknown => BufferSize::Unknown,
            },
            channels: ChannelSpec::Count(format.channels()),
        })
    }

    fn get_name(&self) -> Result<String, InfoError> {
        self.device.name().map_err(|_| InfoError::Unknown)
    }

    fn get_uid(&self) -> Result<String, InfoError> {
        self.device.name().map_err(|_| InfoError::Unknown)
    }

    fn requires_matching_format(&self) -> bool {
        false
    }
}

struct CpalStream<T>
where
    T: GetInnerSamples + SizedSample + Default,
{
    pub ring_buf: rb::Producer<T>,
    pub stream: cpal::Stream,
    pub format: FormatInfo,
}

impl<T> OutputStream for CpalStream<T>
where
    T: GetInnerSamples + SizedSample + Default,
{
    fn submit_frame(&mut self, frame: PlaybackFrame) -> Result<(), SubmissionError> {
        let samples = T::inner(frame.samples);
        let interleaved = interleave(samples);
        let mut slice: &[T] = &interleaved;

        while let Some(written) = self.ring_buf.write_blocking(slice) {
            slice = &slice[written..];
        }

        Ok(())
    }

    fn close_stream(&mut self) -> Result<(), CloseError> {
        Ok(())
    }

    fn needs_input(&self) -> bool {
        true // will always be true as long as the submitting thread is not blocked by submit_frame
    }

    fn get_current_format(&self) -> Result<&FormatInfo, InfoError> {
        Ok(&self.format)
    }
}