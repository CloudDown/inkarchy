//! Local, GPU-only backdrop blur for the eraser's frosted-glass footprint.

use std::sync::{Arc, Mutex};

use eframe::{egui_glow, glow};
use egui::{Color32, PaintCallback, PaintCallbackInfo, Painter, Pos2, Rect, Vec2};
use glow::HasContext;

pub type SharedGlass = Arc<Mutex<Glass>>;

pub struct Glass {
    program: glow::Program,
    vao: glow::VertexArray,
    texture: glow::Texture,
    texture_side: i32,
    backdrop: Option<glow::UniformLocation>,
    capture_origin: Option<glow::UniformLocation>,
    capture_size: Option<glow::UniformLocation>,
    texture_size: Option<glow::UniformLocation>,
    center: Option<glow::UniformLocation>,
    radius: Option<glow::UniformLocation>,
    sigma: Option<glow::UniformLocation>,
    tint: Option<glow::UniformLocation>,
}

impl Glass {
    /// Called once with the application's current GL context.
    pub fn new(gl: &glow::Context) -> Result<SharedGlass, String> {
        unsafe {
            let program = gl.create_program()?;
            let prefix = if gl.version().is_embedded {
                "#version 300 es\nprecision highp float;\nprecision highp int;\n"
            } else {
                "#version 140\n"
            };
            for (kind, source) in [
                (glow::VERTEX_SHADER, include_str!("glass.vert")),
                (glow::FRAGMENT_SHADER, include_str!("glass.frag")),
            ] {
                let shader = match gl.create_shader(kind) {
                    Ok(shader) => shader,
                    Err(error) => {
                        gl.delete_program(program);
                        return Err(error);
                    }
                };
                gl.shader_source(shader, &format!("{prefix}{source}"));
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    let error = gl.get_shader_info_log(shader);
                    gl.delete_shader(shader);
                    gl.delete_program(program);
                    return Err(error);
                }
                gl.attach_shader(program, shader);
                gl.delete_shader(shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                let error = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(error);
            }
            let vao = match gl.create_vertex_array() {
                Ok(vao) => vao,
                Err(error) => {
                    gl.delete_program(program);
                    return Err(error);
                }
            };
            let texture = match gl.create_texture() {
                Ok(texture) => texture,
                Err(error) => {
                    gl.delete_vertex_array(vao);
                    gl.delete_program(program);
                    return Err(error);
                }
            };
            Ok(Arc::new(Mutex::new(Self {
                program,
                vao,
                texture,
                texture_side: 0,
                backdrop: gl.get_uniform_location(program, "backdrop"),
                capture_origin: gl.get_uniform_location(program, "capture_origin"),
                capture_size: gl.get_uniform_location(program, "capture_size"),
                texture_size: gl.get_uniform_location(program, "texture_size"),
                center: gl.get_uniform_location(program, "center"),
                radius: gl.get_uniform_location(program, "radius"),
                sigma: gl.get_uniform_location(program, "sigma"),
                tint: gl.get_uniform_location(program, "tint"),
            })))
        }
    }

    pub fn paint(glass: &SharedGlass, painter: &Painter, center: Pos2, radius: f32, tint: Color32) {
        let glass = Arc::clone(glass);
        painter.add(PaintCallback {
            rect: Rect::from_center_size(center, Vec2::splat(2.0 * (radius + 1.0))),
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                if let Ok(mut glass) = glass.lock() {
                    glass.draw(painter.gl(), info, center, radius, tint);
                }
            })),
        });
    }

    fn draw(
        &mut self,
        gl: &glow::Context,
        info: PaintCallbackInfo,
        pos: Pos2,
        r: f32,
        tint: Color32,
    ) {
        let ppp = info.pixels_per_point;
        let center = egui::pos2(pos.x * ppp, info.screen_size_px[1] as f32 - pos.y * ppp);
        let radius = r * ppp;
        let sigma = (r * 0.16).clamp(1.5, 5.0) * ppp;
        let pad = radius + 3.0 * sigma + 2.0;
        let min = (center - Vec2::splat(pad)).max(Pos2::ZERO).floor();
        let max = (center + Vec2::splat(pad))
            .min(egui::pos2(
                info.screen_size_px[0] as f32,
                info.screen_size_px[1] as f32,
            ))
            .ceil();
        let size = max - min;
        if size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            let side = (size.max_elem() as u32).next_power_of_two() as i32;
            if side > self.texture_side {
                self.texture_side = side;
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    side,
                    side,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::LINEAR as i32);
                }
                for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::CLAMP_TO_EDGE as i32);
                }
            }
            // Copy the already-painted paper/ink, never the desktop or a past
            // frame. The default eframe target is single-sampled.
            gl.copy_tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                min.x as i32,
                min.y as i32,
                size.x as i32,
                size.y as i32,
            );
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.uniform_1_i32(self.backdrop.as_ref(), 0);
            gl.uniform_2_f32(self.capture_origin.as_ref(), min.x, min.y);
            gl.uniform_2_f32(self.capture_size.as_ref(), size.x, size.y);
            gl.uniform_1_f32(self.texture_size.as_ref(), self.texture_side as f32);
            gl.uniform_2_f32(self.center.as_ref(), center.x, center.y);
            gl.uniform_1_f32(self.radius.as_ref(), radius);
            gl.uniform_1_f32(self.sigma.as_ref(), sigma);
            gl.uniform_3_f32(
                self.tint.as_ref(),
                tint.r() as f32 / 255.0,
                tint.g() as f32 / 255.0,
                tint.b() as f32 / 255.0,
            );
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            // egui_glow restores the program, VAO, viewport and blend state.
        }
    }

    /// Called from App::on_exit while the GL context is still current.
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_texture(self.texture);
            gl.delete_vertex_array(self.vao);
            gl.delete_program(self.program);
        }
    }
}
