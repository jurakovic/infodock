//! Raster images with WIC (SPEC §4.6): PNG, JPEG, GIF (first frame), BMP and ICO, converted to
//! premultiplied RGBA and, when a height is requested, scaled with high-quality filtering.
//! Decoding may run on any thread; COM is initialised for the thread on first use.

use std::cell::RefCell;

use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatBmp, GUID_ContainerFormatGif,
    GUID_ContainerFormatIco, GUID_ContainerFormatJpeg, GUID_ContainerFormatPng,
    GUID_VendorMicrosoftBuiltIn, GUID_WICPixelFormat32bppPRGBA, IWICBitmapFrameDecode,
    IWICBitmapSource, IWICImagingFactory, IWICPalette, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeHighQualityCubic, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoUninitialize,
};
use windows::core::GUID;
use windows::core::Interface;

use crate::platform::Bitmap;
use crate::platform::bitmap::{ImageError, ImageFormat, raster_size};

/// COM for this thread and its WIC factory, released when the thread ends.
struct Wic {
    /// COM was initialised here and must be uninitialised.
    initialised: bool,
    factory: Option<IWICImagingFactory>,
}

impl Drop for Wic {
    fn drop(&mut self) {
        self.factory = None;
        if self.initialised {
            // SAFETY: balances the successful CoInitializeEx of this thread.
            unsafe { CoUninitialize() };
        }
    }
}

thread_local! {
    static WIC: RefCell<Option<Wic>> = const { RefCell::new(None) };
}

fn os(e: windows::core::Error) -> ImageError {
    ImageError::Invalid(e.message())
}

fn factory() -> Result<IWICImagingFactory, ImageError> {
    WIC.with(|cell| {
        let mut slot = cell.borrow_mut();
        let wic = slot.get_or_insert_with(|| Wic {
            // A thread that already runs COM in another apartment keeps it (RPC_E_CHANGED_MODE);
            // WIC works in either.
            // SAFETY: plain per-thread initialisation, balanced in `Drop`.
            initialised: unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok()
            },
            factory: None,
        });
        if let Some(factory) = &wic.factory {
            return Ok(factory.clone());
        }
        // SAFETY: creates the in-process WIC factory.
        let factory: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
                .map_err(os)?;
        wic.factory = Some(factory.clone());
        Ok(factory)
    })
}

/// The frame to decode: the first, or for icons the smallest at least `height` tall (else the
/// largest).
fn pick_frame(
    frames: &[(IWICBitmapFrameDecode, u32, u32)],
    height: Option<u32>,
) -> Option<&IWICBitmapFrameDecode> {
    let by_height = |a: &&(IWICBitmapFrameDecode, u32, u32)| (a.2, a.1);
    let largest = frames.iter().max_by_key(by_height);
    let chosen = match height {
        Some(h) => frames
            .iter()
            .filter(|f| f.2 >= h)
            .min_by_key(by_height)
            .or(largest),
        None => largest,
    };
    chosen.map(|f| &f.0)
}

/// The WIC container format of `format`; `None` for SVG, which WIC does not read.
fn container(format: ImageFormat) -> Option<GUID> {
    Some(match format {
        ImageFormat::Png => GUID_ContainerFormatPng,
        ImageFormat::Jpeg => GUID_ContainerFormatJpeg,
        ImageFormat::Gif => GUID_ContainerFormatGif,
        ImageFormat::Bmp => GUID_ContainerFormatBmp,
        ImageFormat::Ico => GUID_ContainerFormatIco,
        ImageFormat::Svg => return None,
    })
}

pub fn decode(
    bytes: &[u8],
    format: ImageFormat,
    height: Option<u32>,
) -> Result<Bitmap, ImageError> {
    let factory = factory()?;
    // SAFETY: plain WIC calls; `bytes` outlives the stream and decoder, which are dropped at the
    // end of this function.
    unsafe {
        let stream = factory.CreateStream().map_err(os)?;
        stream.InitializeFromMemory(bytes).map_err(os)?;
        // The built-in decoder of the sniffed format, never a codec another program installed
        // (SPEC §4.6).
        let container = container(format).ok_or(ImageError::Unsupported)?;
        let decoder = factory
            .CreateDecoder(&container, &GUID_VendorMicrosoftBuiltIn)
            .map_err(os)?;
        decoder
            .Initialize(&stream, WICDecodeMetadataCacheOnDemand)
            .map_err(os)?;
        let count = match format {
            ImageFormat::Ico => decoder.GetFrameCount().map_err(os)?.min(32),
            _ => 1,
        };
        let mut frames = Vec::new();
        for index in 0..count {
            let frame = decoder.GetFrame(index).map_err(os)?;
            let (mut w, mut h) = (0, 0);
            frame.GetSize(&mut w, &mut h).map_err(os)?;
            frames.push((frame, w, h));
        }
        let frame = pick_frame(&frames, height).ok_or(ImageError::Unsupported)?;
        let (mut own_width, mut own_height) = (0, 0);
        frame.GetSize(&mut own_width, &mut own_height).map_err(os)?;
        let (width, height) =
            raster_size(own_width, own_height, height).ok_or(ImageError::TooLarge)?;

        // Premultiply first, so scaling blends colours by their coverage.
        let converter = factory.CreateFormatConverter().map_err(os)?;
        converter
            .Initialize(
                frame,
                &GUID_WICPixelFormat32bppPRGBA,
                WICBitmapDitherTypeNone,
                None::<&IWICPalette>,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(os)?;
        let source: IWICBitmapSource = if (width, height) == (own_width, own_height) {
            converter.cast().map_err(os)?
        } else {
            let scaler = factory.CreateBitmapScaler().map_err(os)?;
            scaler
                .Initialize(
                    &converter,
                    width,
                    height,
                    WICBitmapInterpolationModeHighQualityCubic,
                )
                .map_err(os)?;
            scaler.cast().map_err(os)?
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        source
            .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
            .map_err(os)?;
        Bitmap::from_premultiplied(width, height, pixels).ok_or(ImageError::TooLarge)
    }
}
