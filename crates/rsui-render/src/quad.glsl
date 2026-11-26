// Draws every rectangle and glyph with one pair of shaders, from one list of instances: quad.hlsl, by hand, for Vulkan.
//
// It's the same arithmetic in the same order, so that what Vulkan draws is what Direct3D and the CPU renderer draw. Two
// things differ. Vulkan's clip space has y going down, as the window's pixels do, so the vertex shader doesn't turn it
// over. And there's no ClearType off Windows, so text is never drawn a byte to each part of a pixel, and the blend needs
// only one source.
//
// After changing this, compile it with glslang (glslang-tools on Debian and Ubuntu) and check in what comes out, as the
// program ships the compiled result and never has to load a shader compiler:
//
//     glslangValidator -V --target-env vulkan1.0 -S vert -DVERTEX quad.glsl -o quad_vert.spv
//     glslangValidator -V --target-env vulkan1.0 -S frag -DQUAD quad.glsl -o quad_frag.spv
//     glslangValidator -V --target-env vulkan1.0 -S frag -DIMAGE quad.glsl -o image_frag.spv
//     glslangValidator -V --target-env vulkan1.0 -S frag -DBACKDROP quad.glsl -o backdrop_frag.spv

#version 450

// The same 256 bytes as quad.hlsl's `Frame`, in the same places, written as whole rows of four.
layout(std140, set = 0, binding = 0) uniform Frame {
    // The window's width and height, how many pixels wide and high the atlas is, and how many pixels there are to a unit
    // of what's being drawn.
    vec4 frame_view;
    // Turning and resizing: a point goes to `transform.xy * x + transform.zw * y + moved`.
    vec4 transform;
    // `moved`, then whether any of that's in use.
    vec4 frame_moved;
    // The gamma and contrast glyphs' coverage goes through for their color, as a browser has it. 0 leaves it as it is.
    vec4 frame_text;
    // What CSS's `filter` does to the colors of what's being drawn, then whether there's a filter, and what opacity is
    // multiplied by.
    vec4 filter_red;
    vec4 filter_green;
    vec4 filter_blue;
    vec4 frame_filter;
    // Round corners what's drawn is cut off at, as quad.hlsl has them.
    vec4 round_edges[2];
    vec4 round_top[2];
    vec4 round_bottom[2];
    vec4 round_how[2];
};

#define viewport frame_view.xy
#define atlas_size frame_view.z
#define sharpness frame_view.w
#define moved frame_moved.xy
#define transformed frame_moved.z
#define text_gamma frame_text.x
#define text_contrast frame_text.y
#define filtered frame_filter.x
#define filter_opacity frame_filter.y

// What a shadow inside something is marked with, in place of a glyph's width in the atlas, which is never this big.
const uint INSET_MARK = 65534u;

#ifdef VERTEX

layout(location = 0) in vec4 rect;
layout(location = 1) in uvec4 glyph;
layout(location = 2) in vec4 color;
layout(location = 3) in float radius;

layout(location = 0) flat out vec4 out_color;
layout(location = 1) out vec2 out_local;
layout(location = 2) flat out vec4 out_shape;
layout(location = 3) out vec2 out_uv;
// For a shadow inside something, how far in its edge starts, and 1, or nothing for anything else.
layout(location = 4) flat out vec2 out_inset;
// Where this is before it's turned or resized, then on the screen, in pixels, for what's cut off at round corners.
layout(location = 5) out vec4 out_at;

void main() {
    uint index = uint(gl_VertexIndex);
    vec2 corner = vec2(float(index & 1u), float(index >> 1u));
    vec2 before = rect.xy + corner * rect.zw;
    vec2 position = transform.xy * before.x + transform.zw * before.y + moved;
    out_at = vec4(before, position);
    // The top of the window is -1, as its pixels go down.
    gl_Position = vec4(position / viewport * 2.0 - 1.0, 0.0, 1.0);
    out_color = color;
    out_local = (corner - 0.5) * rect.zw;
    // A rectangle isn't using the atlas, so its first glyph number is how wide its outline is, in sixteenths of a pixel.
    // Zero fills it. A shadow says how soft it is in the second one, which comes through as a negative outline.
    out_shape = vec4(rect.zw / 2.0, radius, (float(glyph.x) - float(glyph.y)) / 16.0);
    // Each of across and down on its own, as HLSL's `?:` on a pair is.
    out_uv = mix(vec2(-1.0), vec2(glyph.xy) + corner * vec2(glyph.zw), greaterThan(glyph.zw, uvec2(0u)));
    out_inset = vec2(0.0);
    if (glyph.z == INSET_MARK) {
        // A shadow inside something: how soft it is, how far in it starts, and how far it's moved, in whole pixels.
        out_shape.w = -float(glyph.y) / 16.0;
        out_uv = vec2(float(glyph.w >> 8u), float(glyph.w & 255u)) - 128.0;
        out_inset = vec2(float(glyph.x) / 16.0, 1.0);
    }
}

#else

layout(location = 0) flat in vec4 in_color;
layout(location = 1) in vec2 in_local;
layout(location = 2) flat in vec4 in_shape;
layout(location = 3) in vec2 in_uv;
layout(location = 4) flat in vec2 in_inset;
layout(location = 5) in vec4 in_at;

layout(location = 0) out vec4 drawn;

// How much of a pixel at `at` is inside a box with round corners, from its edges and how far across and down each corner
// goes, with distances in pixels `sharp` times as big on the screen. A corner whose curve is part of an ellipse is measured
// from that closely enough for its edge to be as smooth.
float round_clip_cover(vec2 at, vec4 edges, vec4 top, vec4 bottom, float sharp) {
    vec2 middle = (edges.xy + edges.zw) / 2.0;
    vec2 half_size = (edges.zw - edges.xy) / 2.0;
    vec2 local = at - middle;
    vec2 curve = local.y < 0.0 ? (local.x < 0.0 ? top.xy : top.zw) : (local.x < 0.0 ? bottom.zw : bottom.xy);
    vec2 away = abs(local) - half_size + curve;
    float distance;
    if (away.x > 0.0 && away.y > 0.0 && curve.x > 0.0 && curve.y > 0.0) {
        float along = length(away / curve);
        distance = along * (along - 1.0) / length(away / (curve * curve));
    } else {
        vec2 out_by = abs(local) - half_size;
        distance = max(out_by.x, out_by.y);
    }
    return clamp(0.5 - distance * sharp, 0.0, 1.0);
}

// How much of this pixel the round clips it's in leave showing.
float round_clips_cover() {
    if (round_how[0].x == 0.0) {
        return 1.0;
    }
    float cover = 1.0;
    for (int at = 0; at < 2; at++) {
        if (round_how[at].x != 0.0) {
            bool on_screen = round_how[at].y > 0.0;
            float inside = round_clip_cover(on_screen ? in_at.zw : in_at.xy, round_edges[at], round_top[at], round_bottom[at],
                on_screen ? 1.0 : sharpness);
            cover *= round_how[at].x > 0.0 ? inside : 1.0 - inside;
        }
    }
    return cover;
}

// A color, through what `filter` says, as a browser has it: on its colors before they're multiplied by how opaque it is.
vec4 filter_color(vec4 color) {
    if (filtered <= 0.0 || color.a <= 0.0) {
        return color;
    }
    vec4 plain = vec4(color.rgb / color.a, 1.0);
    vec3 rgb = clamp(vec3(dot(filter_red, plain), dot(filter_green, plain), dot(filter_blue, plain)), 0.0, 1.0);
    float alpha = color.a * filter_opacity;
    return vec4(rgb * alpha, alpha);
}

#if defined(IMAGE) || defined(BACKDROP)

layout(set = 1, binding = 0) uniform sampler2D picture;

#ifdef BACKDROP
void main() {
    vec2 size = vec2(textureSize(picture, 0));
    float step = min(in_shape.w * sharpness, 4096.0) * 0.75;
    const float weights[4] = float[4](0.30131, 0.22743, 0.09782, 0.024095);
    vec4 color = vec4(0);
    for (int y = -3; y <= 3; ++y) {
        for (int x = -3; x <= 3; ++x) {
            vec2 at = (gl_FragCoord.xy + vec2(x, y) * step) / size;
            color += textureLod(picture, at, 0) * (weights[abs(x)] * weights[abs(y)]);
        }
    }
    vec2 inside = abs(in_local) - in_shape.xy + in_shape.z;
    float distance = length(max(inside, 0.0)) + min(max(inside.x, inside.y), 0.0) - in_shape.z;
    float cover = clamp(0.5 - distance * sharpness, 0.0, 1.0) * round_clips_cover();
    drawn = mix(texelFetch(picture, ivec2(gl_FragCoord.xy), 0), color, cover);
}
#else
// An image reads its texture from where in it this pixel falls, in 65535ths of its size, and its corners can be round. A
// rectangle's outline doesn't apply, so that number means nothing here.
vec4 image_color() {
    vec4 color = in_color * texture(picture, in_uv / 65535.0);
    if (in_shape.z > 0.0 || transformed > 0.0) {
        vec2 inside = abs(in_local) - in_shape.xy + in_shape.z;
        float distance = length(max(inside, 0.0)) + min(max(inside.x, inside.y), 0.0) - in_shape.z;
        color *= clamp(0.5 - distance * sharpness, 0.0, 1.0);
    }
    return color;
}

void main() {
    drawn = filter_color(image_color() * round_clips_cover());
}
#endif

#else

layout(set = 0, binding = 1) uniform sampler2D atlas;

// The error function, which is how much of a bell curve is on one side of a line, to within 0.0000002, as it's worked out
// in Abramowitz and Stegun.
float erf(float x) {
    float t = 1.0 / (1.0 + 0.3275911 * abs(x));
    float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-x * x);
    return x < 0.0 ? -y : y;
}

// A number from 0 to 1 to a power, which is 0 for 0 or less: the logarithm `pow` takes of anything below 0 is NaN, and a
// multiply and add done in one step can leave `1 - src` a hair below 0 (see quad.hlsl).
float gamma_of(float value) {
    return value > 0.0 ? pow(value, text_gamma) : 0.0;
}

// Skia's mask gamma for something this bright, from 0 to 255, which makes how much of a pixel a glyph covers lighter or
// darker for its color the way a browser does it, in eight steps.
float browser_curve(float coverage, float brightness) {
    float step = floor(brightness / 32.0);
    float src = (step * 32.0 + step * 4.0 + floor(step / 2.0)) / 255.0;
    float dst = 1.0 - src;
    float lin_src = gamma_of(src);
    float lin_dst = gamma_of(dst);
    float a = coverage + (1.0 - coverage) * text_contrast * lin_dst * coverage;
    if (abs(src - dst) < 1.0 / 256.0) {
        return roundEven(a * 255.0) / 255.0;
    }
    float lin_out = lin_src * a + (1.0 - a) * lin_dst;
    float out_value = lin_out > 0.0 ? pow(lin_out, 1.0 / text_gamma) : 0.0;
    return roundEven(255.0 * (out_value - dst) / (src - dst)) / 255.0;
}

// How much of a pixel a glyph covers, for its color, by how bright that is, as Skia has it.
float browser_coverage(float coverage, vec4 color) {
    vec3 rgb = color.a > 0.0 ? color.rgb / color.a : color.rgb;
    return browser_curve(coverage, floor(dot(roundEven(rgb * 255.0), vec3(54.0, 183.0, 19.0)) / 256.0));
}

// How much of a pixel a rounded rectangle covers, from how far this pixel is from its middle, its half size, and its radius.
float rounded_cover(vec2 local, vec2 half_size, float radius) {
    vec2 inside = abs(local) - half_size + radius;
    float distance = length(max(inside, 0.0)) + min(max(inside.x, inside.y), 0.0) - radius;
    return clamp(0.5 - distance * sharpness, 0.0, 1.0);
}

// A shadow inside something, as CSS's `inset` has it: all of it, but for a softened hole the shape of what it's in, made
// smaller by its spread and moved by its offset.
vec4 inset_shadow() {
    float reach = -in_shape.w;
    float spread = in_inset.x;
    vec2 local = in_local - in_uv;
    vec2 half_size = max(in_shape.xy - spread, 0.0);
    float radius = max(in_shape.z - spread, 0.0);
    vec2 away = abs(local) - half_size + radius;
    float distance = length(max(away, 0.0)) + min(max(away.x, away.y), 0.0) - radius;
    float hole = reach > 0.0 ? 0.5 * (1.0 - erf(distance / (reach / 3.0 * 1.41421356))) : clamp(0.5 - distance * sharpness, 0.0, 1.0);
    return in_color * rounded_cover(in_local, in_shape.xy, in_shape.z) * (1.0 - hole);
}

// One pixel of the atlas, or nothing outside it, as Direct3D's `Load` gives.
float atlas_at(ivec2 at) {
    ivec2 size = textureSize(atlas, 0);
    if (at.x < 0 || at.y < 0 || at.x >= size.x || at.y >= size.y) {
        return 0.0;
    }
    return texelFetch(atlas, at, 0).r;
}

vec4 color_of() {
    if (in_inset.y > 0.0) {
        return inset_shadow();
    }
    if (in_uv.x >= 0.0) {
        float coverage;
        // Unless they've been turned or resized, when they fall between them.
        if (transformed > 0.0) {
            coverage = texture(atlas, in_uv / atlas_size).r;
        } else {
            // Glyphs line up with whole pixels, so each reads exactly one atlas pixel.
            coverage = atlas_at(ivec2(floor(in_uv)));
        }
        if (text_gamma > 0.0) {
            coverage = browser_coverage(coverage, in_color);
        }
        return in_color * coverage;
    }
    if (in_shape.w < 0.0) {
        // Its rectangle goes as far as it's soft on each side, which is three times the spread of the blur, as a
        // browser's does: what's under a blur that goes from one edge to the other is the area of a bell curve.
        float reach = -in_shape.w;
        vec2 box = in_shape.xy - reach;
        float spread = reach / 3.0 * 1.41421356;
        if (in_shape.z <= 0.0) {
            // Square, so it's the bell curve each way, as exactly as a browser has it.
            vec2 near = (in_local + box) / spread;
            vec2 far = (in_local - box) / spread;
            vec2 each = 0.5 * vec2(erf(near.x) - erf(far.x), erf(near.y) - erf(far.y));
            return in_color * each.x * each.y;
        }
        vec2 away = abs(in_local) - box + in_shape.z;
        float distance = length(max(away, 0.0)) + min(max(away.x, away.y), 0.0) - in_shape.z;
        return in_color * 0.5 * (1.0 - erf(distance / spread));
    }
    // Something that's been turned has edges that don't line up with pixels, however square it is.
    if (in_shape.z > 0.0 || in_shape.w > 0.0 || transformed > 0.0) {
        vec2 inside = abs(in_local) - in_shape.xy + in_shape.z;
        float distance = length(max(inside, 0.0)) + min(max(inside.x, inside.y), 0.0) - in_shape.z;
        float coverage = clamp(0.5 - distance * sharpness, 0.0, 1.0);
        if (in_shape.w > 0.0) {
            coverage *= clamp(0.5 + (distance + in_shape.w) * sharpness, 0.0, 1.0);
        }
        return in_color * coverage;
    }
    return in_color;
}

void main() {
    drawn = filter_color(color_of() * round_clips_cover());
}

#endif
#endif
