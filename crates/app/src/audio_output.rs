use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioOutputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn output_devices() -> Result<Vec<AudioOutputDevice>> {
    Ok(with_system_default(platform::output_devices()?))
}

fn with_system_default(mut devices: Vec<AudioOutputDevice>) -> Vec<AudioOutputDevice> {
    let default_name = devices
        .iter()
        .find(|device| device.is_default)
        .map(|device| device.name.as_str());
    let name = default_name
        .map(|name| format!("System default — {name}"))
        .unwrap_or_else(|| "System default".to_owned());
    devices.insert(
        0,
        AudioOutputDevice {
            id: String::new(),
            name,
            is_default: true,
        },
    );
    devices
}

/// Plays a PCM WAVE cue synchronously on the selected Windows render endpoint.
///
/// An empty endpoint ID follows the current Windows multimedia default. If a saved endpoint was
/// unplugged or removed since it was selected, playback also falls back to that default so alerts
/// are not silently lost.
pub fn play(sound: &'static [u8], endpoint_id: &str) -> Result<()> {
    platform::play(sound, endpoint_id)
}

#[cfg(windows)]
mod platform {
    use super::AudioOutputDevice;
    use anyhow::{anyhow, bail, Context, Result};
    use std::ffi::c_void;
    use std::ptr;
    use std::thread;
    use std::time::Duration;
    use windows::core::{IUnknown, PCWSTR};
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Foundation::{E_FAIL, RPC_E_CHANGED_MODE, S_FALSE, S_OK};
    use windows::Win32::Media::Audio::{
        eMultimedia, eRender, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator,
        MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, WAVEFORMATEX,
    };
    use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToString};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED, STGM_READ,
    };

    const HUNDRED_NS_PER_SECOND: i64 = 10_000_000;
    const REQUESTED_BUFFER_DURATION: i64 = 2_000_000;
    const AUDIO_POLL_INTERVAL: Duration = Duration::from_millis(5);

    struct ComApartment {
        uninitialize: bool,
    }

    impl ComApartment {
        fn enter() -> Result<Self> {
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if result == RPC_E_CHANGED_MODE {
                // The calling thread already owns an STA. Core Audio works in either apartment;
                // only the thread which initialized COM is allowed to uninitialize it.
                return Ok(Self {
                    uninitialize: false,
                });
            }
            result.ok().context("failed initializing Windows COM")?;
            Ok(Self {
                uninitialize: result == S_OK || result == S_FALSE,
            })
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            if self.uninitialize {
                unsafe { CoUninitialize() };
            }
        }
    }

    pub fn output_devices() -> Result<Vec<AudioOutputDevice>> {
        let _apartment = ComApartment::enter()?;
        let enumerator = create_enumerator()?;
        let default_id = unsafe {
            enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .and_then(|device| endpoint_id(&device))
                .ok()
        };
        let collection = unsafe {
            enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .context("failed enumerating active Windows audio outputs")?
        };
        let count = unsafe { collection.GetCount()? };
        let mut devices = Vec::with_capacity(count as usize);
        for index in 0..count {
            let device = unsafe { collection.Item(index)? };
            let id = unsafe { endpoint_id(&device)? };
            let name = unsafe { endpoint_name(&device) }
                .unwrap_or_else(|_| format!("Audio output {}", index + 1));
            devices.push(AudioOutputDevice {
                is_default: default_id.as_deref() == Some(id.as_str()),
                id,
                name,
            });
        }
        devices.sort_by(|left, right| {
            right
                .is_default
                .cmp(&left.is_default)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(devices)
    }

    pub fn play(sound: &'static [u8], requested_endpoint_id: &str) -> Result<()> {
        let wave = PcmWave::parse(sound)?;
        let _apartment = ComApartment::enter()?;
        let enumerator = create_enumerator()?;
        let (device, selected_saved_endpoint) = if requested_endpoint_id.is_empty() {
            (default_endpoint(&enumerator)?, false)
        } else {
            match active_endpoint_by_id(&enumerator, requested_endpoint_id) {
                Ok(device) => (device, true),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        endpoint_id = requested_endpoint_id,
                        "saved audio output is unavailable; using the Windows default"
                    );
                    (default_endpoint(&enumerator)?, false)
                }
            }
        };
        match unsafe { play_on_device(&device, &wave) } {
            Ok(()) => Ok(()),
            Err(failure) if should_retry_default(selected_saved_endpoint, &failure) => {
                tracing::warn!(
                    error = %failure.error,
                    endpoint_id = requested_endpoint_id,
                    "saved audio output failed before playback; retrying on the Windows default"
                );
                let fallback = default_endpoint(&enumerator)?;
                unsafe { play_on_device(&fallback, &wave) }.map_err(|failure| failure.error)
            }
            Err(failure) => Err(failure.error),
        }
    }

    fn create_enumerator() -> Result<IMMDeviceEnumerator> {
        unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None::<&IUnknown>, CLSCTX_ALL)
                .context("failed creating the Windows audio-device enumerator")
        }
    }

    fn default_endpoint(enumerator: &IMMDeviceEnumerator) -> Result<IMMDevice> {
        unsafe {
            enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .context("Windows has no active default audio output")
        }
    }

    fn endpoint_by_id(enumerator: &IMMDeviceEnumerator, endpoint_id: &str) -> Result<IMMDevice> {
        let wide = endpoint_id
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        unsafe {
            enumerator
                .GetDevice(PCWSTR(wide.as_ptr()))
                .context("the selected Windows audio output is unavailable")
        }
    }

    fn active_endpoint_by_id(
        enumerator: &IMMDeviceEnumerator,
        endpoint_id: &str,
    ) -> Result<IMMDevice> {
        let device = endpoint_by_id(enumerator, endpoint_id)?;
        let state = unsafe {
            device
                .GetState()
                .context("could not read the selected Windows audio output state")?
        };
        if !endpoint_state_is_active(state) {
            bail!("the selected Windows audio output is not active");
        }
        Ok(device)
    }

    fn endpoint_state_is_active(state: windows::Win32::Media::Audio::DEVICE_STATE) -> bool {
        state == DEVICE_STATE_ACTIVE
    }

    unsafe fn endpoint_id(device: &IMMDevice) -> windows::core::Result<String> {
        let value = unsafe { device.GetId()? };
        let result = unsafe { value.to_string() }.map_err(|_| {
            windows::core::Error::new(E_FAIL, "audio endpoint ID is not valid UTF-16")
        });
        unsafe { CoTaskMemFree(Some(value.as_ptr().cast::<c_void>())) };
        result
    }

    unsafe fn endpoint_name(device: &IMMDevice) -> Result<String> {
        let store = unsafe { device.OpenPropertyStore(STGM_READ)? };
        let mut value = unsafe { store.GetValue(&PKEY_Device_FriendlyName)? };
        let mut buffer = [0_u16; 512];
        let converted = unsafe { PropVariantToString(&value, &mut buffer) };
        let cleared = unsafe { PropVariantClear(&mut value) };
        converted.context("audio output has no friendly name")?;
        cleared.context("failed releasing an audio-device property")?;
        let length = buffer
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(buffer.len());
        let name = String::from_utf16_lossy(&buffer[..length]);
        if name.trim().is_empty() {
            bail!("audio output has an empty friendly name");
        }
        Ok(name)
    }

    struct PlaybackFailure {
        error: anyhow::Error,
        started: bool,
    }

    impl PlaybackFailure {
        fn before_start(error: anyhow::Error) -> Self {
            Self {
                error,
                started: false,
            }
        }

        fn after_start(error: anyhow::Error) -> Self {
            Self {
                error,
                started: true,
            }
        }
    }

    fn should_retry_default(selected_saved_endpoint: bool, failure: &PlaybackFailure) -> bool {
        selected_saved_endpoint && !failure.started
    }

    unsafe fn play_on_device(
        device: &IMMDevice,
        wave: &PcmWave<'_>,
    ) -> std::result::Result<(), PlaybackFailure> {
        let client: IAudioClient = unsafe {
            device
                .Activate(CLSCTX_ALL, None)
                .context("failed activating the selected Windows audio output")
                .map_err(PlaybackFailure::before_start)?
        };
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                REQUESTED_BUFFER_DURATION,
                0,
                &wave.format,
                None,
            )
        }
        .context("the selected audio output rejected the alert format")
        .map_err(PlaybackFailure::before_start)?;
        let buffer_frames = unsafe { client.GetBufferSize() }
            .context("failed reading the selected audio output buffer size")
            .map_err(PlaybackFailure::before_start)?;
        if buffer_frames == 0 {
            return Err(PlaybackFailure::before_start(anyhow!(
                "the selected audio output provided an empty render buffer"
            )));
        }
        let renderer: IAudioRenderClient = unsafe { client.GetService() }
            .context("failed opening the selected audio output render service")
            .map_err(PlaybackFailure::before_start)?;
        let mut cursor = 0_u32;
        write_available_frames(&client, &renderer, wave, buffer_frames, &mut cursor)
            .map_err(PlaybackFailure::before_start)?;
        unsafe { client.Start() }
            .context("failed starting audio-cue playback")
            .map_err(PlaybackFailure::before_start)?;

        while cursor < wave.frame_count {
            thread::sleep(AUDIO_POLL_INTERVAL);
            write_available_frames(&client, &renderer, wave, buffer_frames, &mut cursor)
                .map_err(PlaybackFailure::after_start)?;
        }
        loop {
            let queued = unsafe { client.GetCurrentPadding() }
                .context("failed reading queued audio-cue frames")
                .map_err(PlaybackFailure::after_start)?;
            if queued == 0 {
                break;
            }
            thread::sleep(AUDIO_POLL_INTERVAL);
        }
        unsafe { client.Stop() }
            .context("failed stopping audio-cue playback")
            .map_err(PlaybackFailure::after_start)?;
        Ok(())
    }

    fn write_available_frames(
        client: &IAudioClient,
        renderer: &IAudioRenderClient,
        wave: &PcmWave<'_>,
        buffer_frames: u32,
        cursor: &mut u32,
    ) -> Result<()> {
        let queued = unsafe { client.GetCurrentPadding()? };
        let available = buffer_frames.saturating_sub(queued);
        let frame_count = available.min(wave.frame_count.saturating_sub(*cursor));
        if frame_count == 0 {
            return Ok(());
        }
        let destination = unsafe { renderer.GetBuffer(frame_count)? };
        let byte_offset = *cursor as usize * wave.format.nBlockAlign as usize;
        let byte_count = frame_count as usize * wave.format.nBlockAlign as usize;
        unsafe {
            ptr::copy_nonoverlapping(
                wave.data[byte_offset..byte_offset + byte_count].as_ptr(),
                destination,
                byte_count,
            );
            renderer.ReleaseBuffer(frame_count, 0)?;
        }
        *cursor += frame_count;
        Ok(())
    }

    struct PcmWave<'a> {
        format: WAVEFORMATEX,
        data: &'a [u8],
        frame_count: u32,
    }

    impl<'a> PcmWave<'a> {
        fn parse(bytes: &'a [u8]) -> Result<Self> {
            if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
                bail!("embedded alert is not a RIFF WAVE file");
            }
            let mut format = None;
            let mut data = None;
            let mut offset = 12_usize;
            while offset.checked_add(8).is_some_and(|end| end <= bytes.len()) {
                let chunk_id = &bytes[offset..offset + 4];
                let chunk_size = u32::from_le_bytes(
                    bytes[offset + 4..offset + 8]
                        .try_into()
                        .expect("four bytes"),
                ) as usize;
                let start = offset + 8;
                let Some(end) = start.checked_add(chunk_size) else {
                    bail!("embedded alert contains an overflowing WAVE chunk");
                };
                if end > bytes.len() {
                    bail!("embedded alert contains a truncated WAVE chunk");
                }
                match chunk_id {
                    b"fmt " if chunk_size >= 16 => {
                        let chunk = &bytes[start..end];
                        let parsed = WAVEFORMATEX {
                            wFormatTag: u16::from_le_bytes(chunk[0..2].try_into().unwrap()),
                            nChannels: u16::from_le_bytes(chunk[2..4].try_into().unwrap()),
                            nSamplesPerSec: u32::from_le_bytes(chunk[4..8].try_into().unwrap()),
                            nAvgBytesPerSec: u32::from_le_bytes(chunk[8..12].try_into().unwrap()),
                            nBlockAlign: u16::from_le_bytes(chunk[12..14].try_into().unwrap()),
                            wBitsPerSample: u16::from_le_bytes(chunk[14..16].try_into().unwrap()),
                            cbSize: 0,
                        };
                        format = Some(parsed);
                    }
                    b"data" => data = Some(&bytes[start..end]),
                    _ => {}
                }
                offset = end + (chunk_size & 1);
            }
            let format = format.context("embedded alert has no WAVE format chunk")?;
            let data = data.context("embedded alert has no WAVE data chunk")?;
            if format.wFormatTag != 1
                || format.nChannels == 0
                || format.nSamplesPerSec == 0
                || format.nBlockAlign == 0
                || format.wBitsPerSample == 0
            {
                bail!("embedded alert is not supported PCM audio");
            }
            if data.len() % format.nBlockAlign as usize != 0 {
                bail!("embedded alert ends with an incomplete PCM frame");
            }
            let frame_count = u32::try_from(data.len() / format.nBlockAlign as usize)
                .context("embedded alert is too long")?;
            if frame_count == 0 {
                bail!("embedded alert contains no PCM frames");
            }
            let expected_average = format.nSamplesPerSec * u32::from(format.nBlockAlign);
            if format.nAvgBytesPerSec != expected_average {
                bail!("embedded alert has an invalid PCM byte rate");
            }
            let duration_hns =
                (i64::from(frame_count) * HUNDRED_NS_PER_SECOND) / i64::from(format.nSamplesPerSec);
            if duration_hns <= 0 {
                bail!("embedded alert duration is invalid");
            }
            Ok(Self {
                format,
                data,
                frame_count,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_both_embedded_alerts_for_wasapi() {
            for sound in [
                include_bytes!("../assets/boss-target-alert.wav").as_slice(),
                include_bytes!("../assets/boss-target-released.wav").as_slice(),
            ] {
                let wave = PcmWave::parse(sound).unwrap();
                let format_tag = wave.format.wFormatTag;
                let channels = wave.format.nChannels;
                let sample_rate = wave.format.nSamplesPerSec;
                let bits_per_sample = wave.format.wBitsPerSample;
                assert_eq!(format_tag, 1);
                assert_eq!(channels, 1);
                assert_eq!(sample_rate, 44_100);
                assert_eq!(bits_per_sample, 16);
                assert!(wave.frame_count > 0);
            }
        }

        #[test]
        fn rejects_truncated_wave_data() {
            let sound = include_bytes!("../assets/boss-target-alert.wav");
            assert!(PcmWave::parse(&sound[..sound.len() - 1]).is_err());
        }

        #[test]
        fn inactive_endpoints_and_prestart_failures_use_the_default_fallback() {
            assert!(endpoint_state_is_active(DEVICE_STATE_ACTIVE));
            assert!(!endpoint_state_is_active(
                windows::Win32::Media::Audio::DEVICE_STATE_DISABLED
            ));

            let before_start = PlaybackFailure::before_start(anyhow!("endpoint disappeared"));
            assert!(should_retry_default(true, &before_start));
            assert!(!should_retry_default(false, &before_start));

            let after_start = PlaybackFailure::after_start(anyhow!("device changed mid-cue"));
            assert!(!should_retry_default(true, &after_start));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_default_is_a_stable_synthetic_choice() {
        let devices = with_system_default(vec![AudioOutputDevice {
            id: "endpoint-id".to_owned(),
            name: "Headphones".to_owned(),
            is_default: true,
        }]);

        assert_eq!(devices[0].id, "");
        assert_eq!(devices[0].name, "System default — Headphones");
        assert_eq!(devices[1].id, "endpoint-id");
    }
}

#[cfg(not(windows))]
mod platform {
    use super::AudioOutputDevice;
    use anyhow::{bail, Result};

    pub fn output_devices() -> Result<Vec<AudioOutputDevice>> {
        Ok(Vec::new())
    }

    pub fn play(_sound: &'static [u8], _endpoint_id: &str) -> Result<()> {
        bail!("selectable audio output is only supported on Windows")
    }
}
