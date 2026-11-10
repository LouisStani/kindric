//! The haze on Wayland: one layer-shell surface per screen, over every window, taking no input,
//! so the mouse and keyboard go through. The haze and the message on top are drawn on the
//! processor once per size, at the screen's scale. The compositor fades it in, as it does every
//! layer surface (Hyprland's `layersIn`): stepping a fade here, buffer after buffer, only made it
//! jerky. Virtual screens (`HEADLESS-…`) are left bare: a skill captures windows there.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use fontdue::{Font, FontSettings};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};

use super::{LIFETIME, haze_alpha, over, premultiplied};

/// How often the process looks at its file: `off` shows within this.
pub const WATCH: Duration = Duration::from_millis(40);
/// The message, in surface pixels: its size, the room around it in its pill, and the pill's
/// distance from the top of the screen.
const TEXT_SIZE: f32 = 15.0;
const PAD_X: u32 = 16;
const PAD_Y: u32 = 8;
const TOP: u32 = 28;
const PILL_OPACITY: f32 = 0.92;
const TEXT_RGB: [u8; 3] = [0xff, 0xf8, 0xef];

/// The text of the message, drawn at one size: how much each pixel is covered, row by row.
struct Label {
    width: u32,
    height: u32,
    coverage: Vec<u8>,
}

struct Screen {
    layer: LayerSurface,
    /// Kept until the surface goes: it says that no input lands on the haze.
    _no_input: Region,
    /// Buffer pixels per surface pixel, from the screen.
    scale: u32,
    /// Size of the buffer, in buffer pixels.
    size: Option<(u32, u32)>,
    drawn: bool,
}

struct Haze {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    pool: SlotPool,
    rgb: [u8; 3],
    message: String,
    font: Option<Font>,
    screens: Vec<Screen>,
    closed: bool,
}

/// Draws the haze in `rgb`, with `message` on top, until `pid_file` goes or is older than
/// `LIFETIME`. The file is written once every screen shows the haze: `on` waits for it.
pub fn show(rgb: [u8; 3], message: &str, pid_file: &Path) -> Result<(), String> {
    let font = match system_font() {
        Ok(font) => Some(font),
        Err(error) => {
            eprintln!("kindric-desktop: the signal shows without its message: {error}");
            None
        },
    };
    let connection = Connection::connect_to_env().map_err(|error| format!("the signal needs Wayland: {error}"))?;
    let (globals, mut queue) = registry_queue_init(&connection).map_err(|error| format!("Wayland: {error}"))?;
    let handle = queue.handle();
    let compositor = CompositorState::bind(&globals, &handle).map_err(|error| format!("Wayland compositor: {error}"))?;
    let layer_shell = LayerShell::bind(&globals, &handle)
        .map_err(|error| format!("this desktop cannot draw over every window (no layer shell): {error}"))?;
    let shm = Shm::bind(&globals, &handle).map_err(|error| format!("Wayland shared memory: {error}"))?;
    let pool = SlotPool::new(4 * 1024 * 1024, &shm).map_err(|error| format!("Wayland shared memory: {error}"))?;
    let mut haze = Haze {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &handle),
        shm,
        pool,
        rgb,
        message: message.to_string(),
        font,
        screens: Vec::new(),
        closed: false,
    };
    // The screens are known after one round trip.
    queue.roundtrip(&mut haze).map_err(|error| format!("Wayland: {error}"))?;
    for output in haze.outputs.outputs() {
        let info = haze.outputs.info(&output);
        if info.as_ref().and_then(|info| info.name.as_deref()).is_some_and(|name| name.starts_with("HEADLESS")) {
            continue;
        }
        let scale = info.map_or(1, |info| u32::try_from(info.scale_factor).unwrap_or(1).max(1));
        let surface = compositor.create_surface(&handle);
        let no_input = Region::new(&compositor).map_err(|error| format!("Wayland region: {error}"))?;
        surface.set_input_region(Some(no_input.wl_region()));
        surface.set_buffer_scale(i32::try_from(scale).unwrap_or(1));
        let layer = layer_shell.create_layer_surface(&handle, surface, Layer::Overlay, Some("kindric-signal"), Some(&output));
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        layer.commit();
        haze.screens.push(Screen { layer, _no_input: no_input, scale, size: None, drawn: false });
    }
    if haze.screens.is_empty() {
        return Err("no screen to show the signal on".to_string());
    }
    let mut announced = false;
    while !haze.closed {
        queue.blocking_dispatch(&mut haze).map_err(|error| format!("Wayland: {error}"))?;
        if !announced && haze.screens.iter().all(|screen| screen.drawn) {
            announced = true;
            announce(pid_file)?;
        }
    }
    Ok(())
}

/// JetBrains Mono bold, the app's font, as fontconfig finds it; else the bold monospace font.
fn system_font() -> Result<Font, String> {
    let path = ["JetBrains Mono:style=Bold", "monospace:style=Bold"]
        .into_iter()
        .filter_map(|pattern| {
            let output = Command::new("fc-match").args(["--format=%{file}", pattern]).output().ok()?;
            let path = String::from_utf8(output.stdout).ok()?;
            (output.status.success() && !path.is_empty()).then_some(path)
        })
        .next()
        .ok_or("fc-match found no font")?;
    let bytes = std::fs::read(&path).map_err(|error| format!("cannot read {path}: {error}"))?;
    Font::from_bytes(bytes, FontSettings::default()).map_err(|error| format!("{path}: {error}"))
}

/// `text` at `size` pixels, on one line.
fn rasterize(font: &Font, text: &str, size: f32) -> Label {
    let (ascent, descent) = font.horizontal_line_metrics(size).map_or((size * 0.8, -size * 0.2), |line| (line.ascent, line.descent));
    let glyphs: Vec<_> = text.chars().map(|character| font.rasterize(character, size)).collect();
    let width = glyphs.iter().map(|(metrics, _)| metrics.advance_width).sum::<f32>().ceil().max(1.0) as u32;
    let height = (ascent - descent).ceil().max(1.0) as u32;
    let mut coverage = vec![0; (width * height) as usize];
    let mut pen = 0.0;
    for (metrics, bitmap) in &glyphs {
        let left = (pen + metrics.xmin as f32).round() as i64;
        let top = (ascent - metrics.height as f32 - metrics.ymin as f32).round() as i64;
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let (x, y) = (left + column as i64, top + row as i64);
                if (0..i64::from(width)).contains(&x) && (0..i64::from(height)).contains(&y) {
                    let index = (y as u32 * width + x as u32) as usize;
                    coverage[index] = coverage[index].max(bitmap[row * metrics.width + column]);
                }
            }
        }
        pen += metrics.advance_width;
    }
    Label { width, height, coverage }
}

/// How much of the pixel centred on (`x`, `y`) a pill of `width` × `height` at (`left`, `top`)
/// covers, its ends fully round.
fn pill_coverage(x: f32, y: f32, left: f32, top: f32, width: f32, height: f32) -> f32 {
    let radius = height / 2.0;
    let dx = ((x - (left + width / 2.0)).abs() - (width / 2.0 - radius)).max(0.0);
    let dy = ((y - (top + height / 2.0)).abs() - (height / 2.0 - radius)).max(0.0);
    (0.5 - ((dx * dx + dy * dy).sqrt() - radius)).clamp(0.0, 1.0)
}

/// The haze on a `width` × `height` buffer at `scale`, and the message's pill at the top.
fn compose(width: u32, height: u32, scale: u32, rgb: [u8; 3], label: Option<&Label>) -> Vec<[u8; 4]> {
    let mut image: Vec<[u8; 4]> =
        (0..height).flat_map(|y| (0..width).map(move |x| premultiplied(rgb, haze_alpha(x, y, width, height)))).collect();
    let Some(label) = label else {
        return image;
    };
    let pill_width = label.width + 2 * PAD_X * scale;
    let pill_height = label.height + 2 * PAD_Y * scale;
    let left = width.saturating_sub(pill_width) / 2;
    let top = TOP * scale;
    for y in top..(top + pill_height).min(height) {
        for x in left..(left + pill_width).min(width) {
            let covered = pill_coverage(x as f32 + 0.5, y as f32 + 0.5, left as f32, top as f32, pill_width as f32, pill_height as f32);
            let index = (y * width + x) as usize;
            image[index] = over(image[index], premultiplied(rgb, PILL_OPACITY * covered));
            let (text_x, text_y) = (x.wrapping_sub(left + PAD_X * scale), y.wrapping_sub(top + PAD_Y * scale));
            if text_x < label.width && text_y < label.height {
                let ink = f32::from(label.coverage[(text_y * label.width + text_x) as usize]) / 255.0;
                image[index] = over(image[index], premultiplied(TEXT_RGB, ink));
            }
        }
    }
    image
}

/// Writes this process's id, then ends the process once `off` removed the file or `LIFETIME`
/// passed since the last `on` wrote it.
fn announce(pid_file: &Path) -> Result<(), String> {
    let own = std::process::id();
    let temporary = pid_file.with_extension("tmp");
    std::fs::write(&temporary, format!("{own}\n"))
        .and_then(|()| std::fs::rename(&temporary, pid_file))
        .map_err(|error| format!("cannot write {}: {error}", pid_file.display()))?;
    let file = pid_file.to_path_buf();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(WATCH);
            if super::read_pid(&file) != Some(own) {
                std::process::exit(0);
            }
            let written = std::fs::metadata(&file).and_then(|metadata| metadata.modified());
            let expired = written.map_or(true, |time| SystemTime::now().duration_since(time).unwrap_or_default() > LIFETIME);
            if expired {
                if super::read_pid(&file) == Some(own) {
                    let _ = std::fs::remove_file(&file);
                }
                std::process::exit(0);
            }
        }
    });
    Ok(())
}

impl Haze {
    /// Draws the haze and the message on the screen of `layer`, at its size.
    fn draw(&mut self, layer: &LayerSurface) {
        let (rgb, font, message) = (self.rgb, self.font.as_ref(), self.message.as_str());
        let Some(screen) = self.screens.iter_mut().find(|screen| &screen.layer == layer) else {
            return;
        };
        let Some((width, height)) = screen.size else {
            return;
        };
        let (Ok(stride), Ok(signed_width), Ok(signed_height)) = (i32::try_from(width * 4), i32::try_from(width), i32::try_from(height)) else {
            return;
        };
        let Ok((buffer, canvas)) = self.pool.create_buffer(signed_width, signed_height, stride, wl_shm::Format::Argb8888) else {
            return;
        };
        let label = font.map(|font| rasterize(font, message, TEXT_SIZE * screen.scale as f32));
        let (pixels, _) = canvas.as_chunks_mut::<4>();
        for (pixel, drawn) in pixels.iter_mut().zip(compose(width, height, screen.scale, rgb, label.as_ref())) {
            *pixel = drawn;
        }
        let surface = screen.layer.wl_surface();
        surface.damage_buffer(0, 0, signed_width, signed_height);
        if buffer.attach_to(surface).is_ok() {
            screen.layer.commit();
            screen.drawn = true;
        }
    }
}

impl CompositorHandler for Haze {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Haze {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Haze {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.screens.retain(|screen| &screen.layer != layer);
        self.closed = self.screens.is_empty();
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        let Some(screen) = self.screens.iter_mut().find(|screen| &screen.layer == layer) else {
            return;
        };
        let (width, height) = (configure.new_size.0 * screen.scale, configure.new_size.1 * screen.scale);
        if width == 0 || height == 0 || screen.size == Some((width, height)) {
            return;
        }
        screen.size = Some((width, height));
        self.draw(layer);
    }
}

impl ShmHandler for Haze {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Haze {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_registry!(Haze);
delegate_dispatch2!(Haze);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_is_solid_inside_round_at_its_ends_and_empty_outside() {
        let (left, top, width, height) = (10.0, 10.0, 100.0, 20.0);
        assert_eq!(pill_coverage(60.0, 20.0, left, top, width, height), 1.0);
        assert_eq!(pill_coverage(60.0, 40.0, left, top, width, height), 0.0);
        assert_eq!(pill_coverage(10.5, 10.5, left, top, width, height), 0.0, "a corner is cut by the round end");
        assert_eq!(pill_coverage(20.0, 20.0, left, top, width, height), 1.0, "the centre of the round end");
    }

    #[test]
    fn the_message_sits_in_its_pill_at_the_top_centre() {
        let label = Label { width: 4, height: 2, coverage: vec![255; 8] };
        let (width, height, scale) = (200, 120, 1);
        let image = compose(width, height, scale, [0xaa, 0x60, 0x1f], Some(&label));
        let pill_width = 4 + 2 * PAD_X;
        let text_left = (width - pill_width) / 2 + PAD_X;
        let text_top = TOP + PAD_Y;
        assert_eq!(image[(text_top * width + text_left) as usize], [0xef, 0xf8, 0xff, 0xff], "text in its own colour");
        let haze_only = compose(width, height, scale, [0xaa, 0x60, 0x1f], None);
        assert_eq!(image[(100 * width + 100) as usize], haze_only[(100 * width + 100) as usize], "the rest is the haze");
    }
}
