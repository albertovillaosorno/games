// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
//
// Boundary-Contract:
// - Owns:
//   - WebGL textures, framebuffer attachments and shader program resources.
// - Must-Not:
//   - Allocate unadmitted pixel data or select portable presentation policy.
// - Allows:
//   - Inputs: Admitted frame layouts, pixels and sprite rectangles.
//   - Outputs: Context-local resource handles and typed operation failures.
//   - Side effects: Allocate and release GPU objects in the owning context.
// - Split-When:
//   - Rendering commands and resource lifetimes need separate ownership.
// - Merge-When:
//   - Shared browser rendering resources cover these same operations.
// - Summary:
//   - Bounded RAII objects for ordered WebGL sprite rendering.
// - Description:
//   - Complete allocation before publishing a resource into a scene.
// - Usage:
//   - Retain resources within one browser rendering session.
// - Defaults:
//   - RGBA8, nearest texel sampling and straight alpha blending.

use games::presentation::{Dimensions, SpriteRect};
use wasm_bindgen::JsCast as _;
use web_sys::{
    WebGl2RenderingContext as Gl, WebGlFramebuffer, WebGlProgram, WebGlShader,
    WebGlTexture, WebGlUniformLocation,
};

use super::super::dom_error;
use crate::ir::frame::{FrameLayout, RenderError};

const VERTEX: &str = r"#version 300 es
precision highp float;
precision highp int;
uniform ivec2 u_target;
uniform ivec4 u_destination;
void main() {
    ivec2 corner = ivec2(gl_VertexID & 1, gl_VertexID >> 1);
    ivec2 position = u_destination.xy + u_destination.zw * corner;
    vec2 unit = vec2(position) / vec2(u_target);
    gl_Position = vec4(unit * vec2(2., -2.) + vec2(-1., 1.), 0., 1.);
}
";

const FRAGMENT: &str = r"#version 300 es
precision highp float;
precision highp int;
uniform ivec2 u_target;
uniform ivec4 u_destination;
uniform ivec4 u_source;
uniform sampler2D u_atlas;
out vec4 color;
void main() {
    vec2 pixel = vec2(gl_FragCoord.x, float(u_target.y)-gl_FragCoord.y);
    vec2 relative = (pixel-vec2(u_destination.xy))/vec2(u_destination.zw);
    ivec2 extent = abs(u_source.zw);
    ivec2 offset = clamp(ivec2(floor(relative*vec2(extent))),
                         ivec2(0), extent-ivec2(1));
    ivec2 texel = u_source.xy + offset * sign(u_source.zw);
    color = texelFetch(u_atlas, texel, 0);
}
";

#[derive(Clone, Copy)]
pub(super) struct Limits {
    max_texture: f64,
    max_viewport: [i32; 2],
}

impl Limits {
    pub(super) fn observe(gl: &Gl) -> Result<Self, RenderError> {
        let texture = gl
            .get_parameter(Gl::MAX_TEXTURE_SIZE)
            .map_err(dom_error)?
            .as_f64()
            .filter(|limit| limit.is_finite() && *limit >= 1.0)
            .ok_or(RenderError::Unsupported)?;
        let viewport = gl
            .get_parameter(Gl::MAX_VIEWPORT_DIMS)
            .map_err(dom_error)?
            .dyn_into::<js_sys::Int32Array>()
            .map_err(|_| RenderError::Unsupported)?;
        if viewport.length() != 2 {
            return Err(RenderError::Unsupported);
        }
        let max_viewport = [viewport.get_index(0), viewport.get_index(1)];
        if max_viewport.iter().any(|limit| *limit <= 0) {
            return Err(RenderError::Unsupported);
        }
        checked(gl)?;
        Ok(Self {
            max_texture: texture,
            max_viewport,
        })
    }

    fn texture(self, dimensions: Dimensions) -> Result<[i32; 2], RenderError> {
        if f64::from(dimensions.width.get()) > self.max_texture
            || f64::from(dimensions.height.get()) > self.max_texture
        {
            return Err(RenderError::TooLarge);
        }
        size(dimensions)
    }

    pub(super) fn viewport(
        self,
        dimensions: Dimensions,
    ) -> Result<[i32; 2], RenderError> {
        let [width, height] = size(dimensions)?;
        let [max_width, max_height] = self.max_viewport;
        if width > max_width || height > max_height {
            return Err(RenderError::TooLarge);
        }
        Ok([width, height])
    }
}

pub(super) struct Texture {
    gl: Gl,
    handle: WebGlTexture,
}

impl Texture {
    pub(super) fn new(
        gl: &Gl,
        limits: Limits,
        layout: FrameLayout,
        pixels: Option<&[u8]>,
    ) -> Result<Self, RenderError> {
        let [width, height] = limits.texture(dimensions(layout))?;
        let texture = Self {
            gl: gl.clone(),
            handle: gl.create_texture().ok_or(RenderError::TooLarge)?,
        };
        texture.bind();
        let nearest = signed(Gl::NEAREST)?;
        let clamp = signed(Gl::CLAMP_TO_EDGE)?;
        for (parameter, value) in [
            (Gl::TEXTURE_MIN_FILTER, nearest),
            (Gl::TEXTURE_MAG_FILTER, nearest),
            (Gl::TEXTURE_WRAP_S, clamp),
            (Gl::TEXTURE_WRAP_T, clamp),
        ] {
            gl.tex_parameteri(Gl::TEXTURE_2D, parameter, value);
        }
        gl.tex_storage_2d(Gl::TEXTURE_2D, 1, Gl::RGBA8, width, height);
        checked(gl)?;
        if let Some(pixels) = pixels {
            upload(gl, width, height, pixels)?;
        }
        checked(gl)?;
        Ok(texture)
    }

    pub(super) fn bind(&self) {
        self.gl.bind_texture(Gl::TEXTURE_2D, Some(&self.handle));
    }

    pub(super) fn replace(
        &self,
        layout: FrameLayout,
        pixels: &[u8],
    ) -> Result<(), RenderError> {
        checked(&self.gl)?;
        self.bind();
        let [width, height] = size(dimensions(layout))?;
        upload(&self.gl, width, height, pixels)?;
        checked(&self.gl)
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        self.gl.delete_texture(Some(&self.handle));
    }
}

pub(super) struct Target {
    gl: Gl,
    framebuffer: WebGlFramebuffer,
    texture: Texture,
}

impl Target {
    pub(super) fn new(
        gl: &Gl,
        limits: Limits,
        layout: FrameLayout,
    ) -> Result<Self, RenderError> {
        let _viewport = limits.viewport(dimensions(layout))?;
        let texture = Texture::new(gl, limits, layout, None)?;
        let target = Self {
            gl: gl.clone(),
            framebuffer: gl
                .create_framebuffer()
                .ok_or(RenderError::TooLarge)?,
            texture,
        };
        target.bind(Gl::FRAMEBUFFER);
        gl.framebuffer_texture_2d(
            Gl::FRAMEBUFFER,
            Gl::COLOR_ATTACHMENT0,
            Gl::TEXTURE_2D,
            Some(&target.texture.handle),
            0,
        );
        if gl.check_framebuffer_status(Gl::FRAMEBUFFER)
            != Gl::FRAMEBUFFER_COMPLETE
        {
            return Err(RenderError::Backend(
                "browser logical framebuffer incomplete".to_owned(),
            ));
        }
        checked(gl)?;
        Ok(target)
    }

    pub(super) fn bind(&self, target: u32) {
        self.gl.bind_framebuffer(target, Some(&self.framebuffer));
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        self.gl.delete_framebuffer(Some(&self.framebuffer));
    }
}

struct Shader {
    gl: Gl,
    handle: WebGlShader,
}

impl Shader {
    fn compile(gl: &Gl, kind: u32, source: &str) -> Result<Self, RenderError> {
        let shader = Self {
            gl: gl.clone(),
            handle: gl.create_shader(kind).ok_or(RenderError::Unsupported)?,
        };
        gl.shader_source(&shader.handle, source);
        gl.compile_shader(&shader.handle);
        if gl
            .get_shader_parameter(&shader.handle, Gl::COMPILE_STATUS)
            .as_bool()
            != Some(true)
        {
            return Err(RenderError::Backend(
                "browser sprite shader compilation failed".to_owned(),
            ));
        }
        checked(gl)?;
        Ok(shader)
    }
}

impl Drop for Shader {
    fn drop(&mut self) {
        self.gl.delete_shader(Some(&self.handle));
    }
}

struct ProgramHandle {
    gl: Gl,
    handle: WebGlProgram,
}

impl Drop for ProgramHandle {
    fn drop(&mut self) {
        self.gl.use_program(None);
        self.gl.delete_program(Some(&self.handle));
    }
}

pub(super) struct Program {
    raw: ProgramHandle,
    target: WebGlUniformLocation,
    destination: WebGlUniformLocation,
    source: WebGlUniformLocation,
    atlas: WebGlUniformLocation,
}

impl Program {
    pub(super) fn new(gl: &Gl) -> Result<Self, RenderError> {
        let vertex = Shader::compile(gl, Gl::VERTEX_SHADER, VERTEX)?;
        let fragment = Shader::compile(gl, Gl::FRAGMENT_SHADER, FRAGMENT)?;
        let raw = ProgramHandle {
            gl: gl.clone(),
            handle: gl.create_program().ok_or(RenderError::Unsupported)?,
        };
        gl.attach_shader(&raw.handle, &vertex.handle);
        gl.attach_shader(&raw.handle, &fragment.handle);
        gl.link_program(&raw.handle);
        if gl
            .get_program_parameter(&raw.handle, Gl::LINK_STATUS)
            .as_bool()
            != Some(true)
        {
            return Err(RenderError::Backend(
                "browser sprite program linking failed".to_owned(),
            ));
        }
        gl.detach_shader(&raw.handle, &vertex.handle);
        gl.detach_shader(&raw.handle, &fragment.handle);
        let uniform = |name: &str| {
            gl.get_uniform_location(&raw.handle, name)
                .ok_or(RenderError::Unsupported)
        };
        let target = uniform("u_target")?;
        let destination = uniform("u_destination")?;
        let source = uniform("u_source")?;
        let atlas = uniform("u_atlas")?;
        checked(gl)?;
        Ok(Self {
            raw,
            target,
            destination,
            source,
            atlas,
        })
    }

    pub(super) fn activate(&self, size: [i32; 2]) {
        let gl = &self.raw.gl;
        let [width, height] = size;
        gl.use_program(Some(&self.raw.handle));
        gl.active_texture(Gl::TEXTURE0);
        gl.uniform1i(Some(&self.atlas), 0);
        gl.uniform2i(Some(&self.target), width, height);
    }

    pub(super) fn draw(
        &self,
        sprite: crate::ir::scene::SpriteDraw,
    ) -> Result<(), RenderError> {
        let gl = &self.raw.gl;
        let [x, y, width, height] = rectangle(sprite.destination)?;
        gl.uniform4i(Some(&self.destination), x, y, width, height);
        let [x, y, width, height] = rectangle(sprite.source)?;
        let (x, width) = flipped(x, width, sprite.flip_horizontal)?;
        let (y, height) = flipped(y, height, sprite.flip_vertical)?;
        gl.uniform4i(Some(&self.source), x, y, width, height);
        gl.draw_arrays(Gl::TRIANGLE_STRIP, 0, 4);
        Ok(())
    }
}

pub(super) fn checked(gl: &Gl) -> Result<(), RenderError> {
    if gl.is_context_lost() {
        return Err(RenderError::Backend(
            "browser WebGL context lost".to_owned(),
        ));
    }
    match gl.get_error() {
        Gl::NO_ERROR => Ok(()),
        Gl::OUT_OF_MEMORY => Err(RenderError::TooLarge),
        error => Err(RenderError::Backend(format!(
            "browser WebGL operation failed: {error}"
        ))),
    }
}

pub(super) const fn dimensions(layout: FrameLayout) -> Dimensions {
    Dimensions::new(layout.width(), layout.height())
}

fn flipped(
    origin: i32,
    extent: i32,
    flip: bool,
) -> Result<(i32, i32), RenderError> {
    if flip {
        let last = origin
            .checked_add(extent - 1)
            .ok_or(RenderError::InvalidDimensions)?;
        Ok((last, -extent))
    } else {
        Ok((origin, extent))
    }
}

fn rectangle(rect: SpriteRect) -> Result<[i32; 4], RenderError> {
    Ok([
        rect.x,
        rect.y,
        signed(rect.width.get())?,
        signed(rect.height.get())?,
    ])
}

pub(super) fn signed(value: u32) -> Result<i32, RenderError> {
    i32::try_from(value).map_err(|_| RenderError::InvalidDimensions)
}

fn size(dimensions: Dimensions) -> Result<[i32; 2], RenderError> {
    Ok([
        signed(dimensions.width.get())?,
        signed(dimensions.height.get())?,
    ])
}

fn upload(
    gl: &Gl,
    width: i32,
    height: i32,
    pixels: &[u8],
) -> Result<(), RenderError> {
    gl.tex_sub_image_2d_with_i32_and_i32_and_u32_and_type_and_opt_u8_array(
        Gl::TEXTURE_2D,
        0,
        0,
        0,
        width,
        height,
        Gl::RGBA,
        Gl::UNSIGNED_BYTE,
        Some(pixels),
    )
    .map_err(dom_error)
}
