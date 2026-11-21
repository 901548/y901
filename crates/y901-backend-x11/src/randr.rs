//! Minimal raw XRandR 1.3+ requests and replies for physical output DPI discovery.
//!
//! This module deliberately implements only the small subset needed to enumerate
//! current outputs and read their physical dimensions/current CRTC mode. It has no
//! X11 binding dependencies.

use crate::{get_u16, get_u32, put_u16, put_u32};

/// XRandR extension name used with the core `QueryExtension` request.
pub const EXTENSION_NAME: &[u8] = b"RANDR";
/// XRandR protocol version required by `GetScreenResourcesCurrent`.
pub const REQUIRED_MAJOR: u32 = 1;
/// XRandR 1.3 introduced current screen resources.
pub const REQUIRED_MINOR: u32 = 3;

const QUERY_VERSION: u8 = 0;
const GET_OUTPUT_INFO: u8 = 9;
const GET_CRTC_INFO: u8 = 20;
const GET_SCREEN_RESOURCES_CURRENT: u8 = 25;
const STATUS_SUCCESS: u8 = 0;
const CONNECTION_CONNECTED: u8 = 0;

/// Physical geometry and density for one active XRandR output.
///
/// `x`/`y`/`width`/`height` describe the CRTC's position and size in root-window
/// pixels, which makes it possible to map a window position to the monitor that
/// contains it. `dpi_x`/`dpi_y` are physical densities derived from that CRTC
/// size and the output's millimeter size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputDpi {
    /// XRandR output resource identifier.
    pub output: u32,
    /// CRTC left edge in root-window pixels.
    pub x: i16,
    /// CRTC top edge in root-window pixels.
    pub y: i16,
    /// CRTC width in pixels.
    pub width: u16,
    /// CRTC height in pixels.
    pub height: u16,
    /// Horizontal pixel density, computed from the active CRTC width and output millimeters.
    pub dpi_x: f32,
    /// Vertical pixel density, computed from the active CRTC height and output millimeters.
    pub dpi_y: f32,
}

impl OutputDpi {
    /// Whether a root-window pixel coordinate falls inside this output's CRTC.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        let (x0, y0) = (self.x as i32, self.y as i32);
        x >= x0 && y >= y0 && x < x0 + self.width as i32 && y < y0 + self.height as i32
    }
}

/// Physical DPI of the output containing the given root-window pixel, if any.
///
/// Returns `None` when no output covers the point; callers should decide whether
/// that means "use a global fallback" or "unknown".
pub fn dpi_at(outputs: &[OutputDpi], x: i32, y: i32) -> Option<(f32, f32)> {
    outputs
        .iter()
        .find(|output| output.contains(x, y))
        .map(|output| (output.dpi_x, output.dpi_y))
}

/// Typed XRandR QueryVersion reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionReply {
    /// Server-supported major version.
    pub major: u32,
    /// Server-supported minor version.
    pub minor: u32,
}

/// Typed GetScreenResourcesCurrent reply fields needed for output enumeration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenResourcesReply {
    /// Server configuration timestamp, used to validate subsequent output queries.
    pub config_timestamp: u32,
    /// Output resource identifiers in server order.
    pub outputs: Vec<u32>,
}

/// Typed GetOutputInfo reply fields needed for active-output discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputInfoReply {
    /// Whether the output is currently connected.
    pub connected: bool,
    /// The currently attached CRTC, or zero when inactive.
    pub crtc: u32,
    /// Physical width reported by the server, in millimeters.
    pub width_mm: u32,
    /// Physical height reported by the server, in millimeters.
    pub height_mm: u32,
}

/// Typed GetCrtcInfo reply fields needed for active mode dimensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrtcInfoReply {
    /// CRTC left edge in root-window pixels.
    pub x: i16,
    /// CRTC top edge in root-window pixels.
    pub y: i16,
    /// Current CRTC mode width in pixels.
    pub width: u16,
    /// Current CRTC mode height in pixels.
    pub height: u16,
    /// Outputs attached to this CRTC.
    pub outputs: Vec<u32>,
}

/// Encode XRandR `QueryVersion` (minor opcode 0) for the given extension opcode.
pub fn encode_query_version(major_opcode: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&[major_opcode, QUERY_VERSION]);
    put_u16(&mut out, 3);
    put_u32(&mut out, REQUIRED_MAJOR);
    put_u32(&mut out, REQUIRED_MINOR);
    out
}

/// Encode `GetScreenResourcesCurrent` (minor opcode 25).
pub fn encode_get_screen_resources_current(major_opcode: u8, root: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&[major_opcode, GET_SCREEN_RESOURCES_CURRENT]);
    put_u16(&mut out, 2);
    put_u32(&mut out, root);
    out
}

/// Encode `GetOutputInfo` (minor opcode 9).
pub fn encode_get_output_info(major_opcode: u8, output: u32, config_timestamp: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&[major_opcode, GET_OUTPUT_INFO]);
    put_u16(&mut out, 3);
    put_u32(&mut out, output);
    put_u32(&mut out, config_timestamp);
    out
}

/// Encode `GetCrtcInfo` (minor opcode 20).
pub fn encode_get_crtc_info(major_opcode: u8, crtc: u32, config_timestamp: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&[major_opcode, GET_CRTC_INFO]);
    put_u16(&mut out, 3);
    put_u32(&mut out, crtc);
    put_u32(&mut out, config_timestamp);
    out
}

/// Parse the fixed portion of a QueryVersion reply.
pub fn parse_version_reply(reply: &[u8]) -> Result<VersionReply, String> {
    require_reply(reply, "QueryVersion")?;
    Ok(VersionReply {
        major: get_u32(reply, 8).ok_or("truncated XRandR version major")?,
        minor: get_u32(reply, 12).ok_or("truncated XRandR version minor")?,
    })
}

/// Parse a GetScreenResourcesCurrent reply including its variable output list.
pub fn parse_screen_resources_reply(
    reply: &[u8],
    extra: &[u8],
) -> Result<ScreenResourcesReply, String> {
    require_reply(reply, "GetScreenResourcesCurrent")?;
    let n_crtcs = get_u16(reply, 16).ok_or("truncated XRandR CRTC count")? as usize;
    let n_outputs = get_u16(reply, 18).ok_or("truncated XRandR output count")? as usize;
    let n_modes = get_u16(reply, 20).ok_or("truncated XRandR mode count")? as usize;
    let n_names = get_u16(reply, 22).ok_or("truncated XRandR name length")? as usize;
    let output_offset = n_crtcs.checked_mul(4).ok_or("XRandR CRTC list overflow")?;
    let output_bytes = n_outputs
        .checked_mul(4)
        .ok_or("XRandR output list overflow")?;
    // Each XRRModeInfo occupies 32 bytes in the wire reply.
    let modes_bytes = n_modes.checked_mul(32).ok_or("XRandR mode list overflow")?;
    let required = output_offset
        .checked_add(output_bytes)
        .and_then(|n| n.checked_add(modes_bytes))
        .and_then(|n| n.checked_add(n_names))
        .ok_or("XRandR resource data overflow")?;
    if extra.len() < required {
        return Err(format!(
            "truncated XRandR resources data: need {required} bytes, got {}",
            extra.len()
        ));
    }
    let outputs = (0..n_outputs)
        .map(|i| {
            get_u32(extra, output_offset + i * 4)
                .ok_or_else(|| "truncated XRandR output id".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ScreenResourcesReply {
        config_timestamp: get_u32(reply, 12).ok_or("truncated XRandR config timestamp")?,
        outputs,
    })
}

/// Parse a GetOutputInfo reply. Non-success status is reported as an error.
pub fn parse_output_info_reply(reply: &[u8]) -> Result<OutputInfoReply, String> {
    require_reply(reply, "GetOutputInfo")?;
    if reply[1] != STATUS_SUCCESS {
        return Err(format!("XRandR GetOutputInfo status {}", reply[1]));
    }
    Ok(OutputInfoReply {
        crtc: get_u32(reply, 12).ok_or("truncated XRandR output CRTC")?,
        width_mm: get_u32(reply, 16).ok_or("truncated XRandR output width")?,
        height_mm: get_u32(reply, 20).ok_or("truncated XRandR output height")?,
        connected: reply[24] == CONNECTION_CONNECTED,
    })
}

/// Parse a GetCrtcInfo reply including attached output ids. Non-success is an error.
pub fn parse_crtc_info_reply(reply: &[u8], extra: &[u8]) -> Result<CrtcInfoReply, String> {
    require_reply(reply, "GetCrtcInfo")?;
    if reply[1] != STATUS_SUCCESS {
        return Err(format!("XRandR GetCrtcInfo status {}", reply[1]));
    }
    let n_outputs = get_u16(reply, 28).ok_or("truncated XRandR CRTC output count")? as usize;
    let required = n_outputs
        .checked_mul(4)
        .ok_or("XRandR CRTC output list overflow")?;
    if extra.len() < required {
        return Err(format!(
            "truncated XRandR CRTC outputs: need {required} bytes, got {}",
            extra.len()
        ));
    }
    let outputs = (0..n_outputs)
        .map(|i| get_u32(extra, i * 4).ok_or_else(|| "truncated XRandR CRTC output id".to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CrtcInfoReply {
        x: get_u16(reply, 12).ok_or("truncated XRandR CRTC x")? as i16,
        y: get_u16(reply, 14).ok_or("truncated XRandR CRTC y")? as i16,
        width: get_u16(reply, 16).ok_or("truncated XRandR CRTC width")?,
        height: get_u16(reply, 18).ok_or("truncated XRandR CRTC height")?,
        outputs,
    })
}

fn require_reply(reply: &[u8], name: &str) -> Result<(), String> {
    if reply.len() < 32 {
        return Err(format!(
            "truncated XRandR {name} reply: {} bytes",
            reply.len()
        ));
    }
    if reply[0] != 1 {
        return Err(format!("invalid XRandR {name} reply type {}", reply[0]));
    }
    Ok(())
}
