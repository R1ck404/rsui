// Draws every rectangle and glyph with one pair of shaders, from one list of instances.
//
// After changing this, run `cargo run -p rsui-render --example compile_shaders`. The
// program ships the compiled result, so it never has to load the shader compiler.

cbuffer Frame : register(b0) {
    float2 viewport;
    // How many pixels wide and high the atlas is, for when it's read smoothly.
    float atlas_size;
    // How many pixels there are to a unit of what's being drawn, which is 1 unless it's been resized.
    float sharpness;
    // Turning and resizing: a point goes to `transform.xy * x + transform.zw * y + moved`.
    float4 transform;
    float2 moved;
    // Whether any of that's in use, so that things can't line up with whole pixels.
    float transformed;
    float unused;
    // The gamma and contrast glyphs' coverage goes through for their color, as a browser has it. 0 leaves it as it is.
    float text_gamma;
    float text_contrast;
    float2 unused_too;
    // What CSS's `filter` does to the colors of what's being drawn: each of red, green and blue from those, as a row of what each
    // is multiplied by and what's added, then what its opacity is multiplied by, if `filtered` says there's a filter at all.
    float4 filter_red;
    float4 filter_green;
    float4 filter_blue;
    float filtered;
    float filter_opacity;
    float2 unused_three;
    // Round corners what's drawn is cut off at, as CSS's `overflow: hidden` with `border-radius` has it, or a box with round
    // corners it's kept out of, as a shadow is from under what it's behind, filled from the first: each one's left, top, right
    // and bottom edges, how far across and down its top left and top right corners go, then its bottom right and bottom left,
    // and whether it keeps what's inside (1) or outside (-1) or isn't there (0), and whether it's measured on the screen (1),
    // rather than before what's drawn is turned or resized (0).
    float4 round_edges[2];
    float4 round_top[2];
    float4 round_bottom[2];
    float4 round_how[2];
};

Texture2D<float> atlas : register(t0);
Texture2D picture : register(t1);
SamplerState smooth : register(s0);

struct Instance {
    float4 rect : RECT;
    uint4 glyph : GLYPH;
    float4 color : COLOR;
    float radius : RADIUS;
    uint corner : SV_VertexID;
};

struct Pixel {
    float4 position : SV_Position;
    nointerpolation float4 color : COLOR;
    float2 local : LOCAL;
    nointerpolation float4 shape : SHAPE;
    float2 uv : UV;
    // For a shadow inside something, how far in its edge starts, and 1, or nothing for anything else.
    nointerpolation float2 inset : INSET;
    // Where this is before it's turned or resized, then on the screen, in pixels, for what's cut off at round corners.
    float4 at : AT;
};

// What a shadow inside something is marked with, in place of a glyph's width in the atlas, which is never this big.
static const uint INSET_MARK = 65534;

Pixel vs(Instance input) {
    float2 corner = float2(input.corner & 1, input.corner >> 1);
    float2 before = input.rect.xy + corner * input.rect.zw;
    float2 position = transform.xy * before.x + transform.zw * before.y + moved;
    Pixel output;
    output.at = float4(before, position);
    output.position = float4(position / viewport * float2(2, -2) + float2(-1, 1), 0, 1);
    output.color = input.color;
    output.local = (corner - 0.5) * input.rect.zw;
    // A rectangle isn't using the atlas, so its first glyph number is how wide its outline
    // is, in sixteenths of a pixel. Zero fills it.
    // A shadow says how soft it is in the second one, which comes through as a negative outline.
    output.shape = float4(input.rect.zw / 2, input.radius, (float(input.glyph.x) - float(input.glyph.y)) / 16.0);
    output.uv = input.glyph.zw > 0 ? input.glyph.xy + corner * input.glyph.zw : float2(-1, -1);
    output.inset = float2(0, 0);
    if (input.glyph.z == INSET_MARK) {
        // A shadow inside something: how soft it is, how far in it starts, and how far it's moved, in whole pixels.
        output.shape.w = -float(input.glyph.y) / 16.0;
        output.uv = float2(float(input.glyph.w >> 8), float(input.glyph.w & 255)) - 128;
        output.inset = float2(float(input.glyph.x) / 16.0, 1);
    }
    return output;
}

// An image reads its texture from where in it this pixel falls, in 65535ths of its size, and its
// corners can be round. A rectangle's outline doesn't apply, so that number means nothing here.
float4 image_color(Pixel input) {
    float4 color = input.color * picture.Sample(smooth, input.uv / 65535.0);
    if (input.shape.z > 0 || transformed > 0) {
        float2 inside = abs(input.local) - input.shape.xy + input.shape.z;
        float distance = length(max(inside, 0)) + min(max(inside.x, inside.y), 0) - input.shape.z;
        color *= saturate(0.5 - distance * sharpness);
    }
    return color;
}

// The error function, which is how much of a bell curve is on one side of a line, to within 0.0000002, as it's worked out in
// Abramowitz and Stegun.
float erf(float x) {
    float t = 1 / (1 + 0.3275911 * abs(x));
    float y = 1 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-x * x);
    return x < 0 ? -y : y;
}

// A number from 0 to 1 to a power, which is 0 for 0 or less. `pow` is a logarithm and back, and the logarithm of anything below 0
// is NaN: the compiler makes `1 - src` a multiply and add, which a real GPU does in one step, so that for the brightest colors it
// comes out a hair below 0, where Windows' software GPU rounds it to exactly 0. A NaN there left light text on a dark background
// with nothing drawn at all, in a window but not headless.
float gamma_of(float value) {
    return value > 0 ? pow(value, text_gamma) : 0;
}

// Skia's mask gamma for something this bright, from 0 to 255, which makes how much of a pixel a glyph covers lighter or darker for
// its color the way a browser does it, in eight steps.
float browser_curve(float coverage, float brightness) {
    float step = floor(brightness / 32);
    float src = (step * 32 + step * 4 + floor(step / 2)) / 255;
    float dst = 1 - src;
    float lin_src = gamma_of(src);
    float lin_dst = gamma_of(dst);
    float a = coverage + (1 - coverage) * text_contrast * lin_dst * coverage;
    if (abs(src - dst) < 1.0 / 256) {
        return round(a * 255) / 255;
    }
    float lin_out = lin_src * a + (1 - a) * lin_dst;
    float out_value = lin_out > 0 ? pow(lin_out, 1 / text_gamma) : 0;
    return round(255 * (out_value - dst) / (src - dst)) / 255;
}

// How much of a pixel a glyph covers, for its color, by how bright that is, as Skia has it.
float browser_coverage(float coverage, float4 color) {
    float3 rgb = color.a > 0 ? color.rgb / color.a : color.rgb;
    return browser_curve(coverage, floor(dot(round(rgb * 255), float3(54, 183, 19)) / 256));
}

// The same for one part of a pixel, as ClearType's are, by how bright that part of the color is.
float browser_part(float coverage, float part) {
    return browser_curve(coverage, round(part * 255));
}

// How much of a pixel a rounded rectangle covers, from how far this pixel is from its middle, its half size, and its radius.
float rounded_cover(float2 local, float2 half_size, float radius) {
    float2 inside = abs(local) - half_size + radius;
    float distance = length(max(inside, 0)) + min(max(inside.x, inside.y), 0) - radius;
    return saturate(0.5 - distance * sharpness);
}

// A shadow inside something, as CSS's `inset` has it: all of it, but for a softened hole the shape of what it's in, made
// smaller by its spread and moved by its offset.
float4 inset_shadow(Pixel input) {
    float reach = -input.shape.w;
    float spread = input.inset.x;
    float2 local = input.local - input.uv;
    float2 half_size = max(input.shape.xy - spread, 0);
    float radius = max(input.shape.z - spread, 0);
    float2 away = abs(local) - half_size + radius;
    float distance = length(max(away, 0)) + min(max(away.x, away.y), 0) - radius;
    float hole = reach > 0 ? 0.5 * (1 - erf(distance / (reach / 3 * 1.41421356))) : saturate(0.5 - distance * sharpness);
    return input.color * rounded_cover(input.local, input.shape.xy, input.shape.z) * (1 - hole);
}

// How much of a pixel at `at` is inside a box with round corners, from its edges and how far across and down each corner goes,
// with distances in pixels `sharp` times as big on the screen. A corner whose curve is part of an ellipse is measured from that
// closely enough for its edge to be as smooth.
float round_clip_cover(float2 at, float4 edges, float4 top, float4 bottom, float sharp) {
    float2 middle = (edges.xy + edges.zw) / 2;
    float2 half_size = (edges.zw - edges.xy) / 2;
    float2 local = at - middle;
    float2 curve = local.y < 0 ? (local.x < 0 ? top.xy : top.zw) : (local.x < 0 ? bottom.zw : bottom.xy);
    float2 away = abs(local) - half_size + curve;
    float distance;
    if (away.x > 0 && away.y > 0 && curve.x > 0 && curve.y > 0) {
        float along = length(away / curve);
        distance = along * (along - 1) / length(away / (curve * curve));
    } else {
        float2 out_by = abs(local) - half_size;
        distance = max(out_by.x, out_by.y);
    }
    return saturate(0.5 - distance * sharp);
}

// How much of this pixel the round clips it's in leave showing.
float round_clips_cover(Pixel input) {
    if (round_how[0].x == 0) {
        return 1;
    }
    float cover = 1;
    [unroll] for (int at = 0; at < 2; at++) {
        if (round_how[at].x != 0) {
            bool on_screen = round_how[at].y > 0;
            float inside = round_clip_cover(on_screen ? input.at.zw : input.at.xy, round_edges[at], round_top[at],
                round_bottom[at], on_screen ? 1 : sharpness);
            cover *= round_how[at].x > 0 ? inside : 1 - inside;
        }
    }
    return cover;
}

float4 color_of(Pixel input) {
    if (input.inset.y > 0) {
        return inset_shadow(input);
    }
    if (input.uv.x >= 0) {
        float coverage;
        // Unless they've been turned or resized, when they fall between them.
        if (transformed > 0) {
            coverage = atlas.Sample(smooth, input.uv / atlas_size);
        } else {
            // Glyphs line up with whole pixels, so each reads exactly one atlas pixel.
            coverage = atlas.Load(int3(floor(input.uv), 0));
        }
        if (text_gamma > 0) {
            coverage = browser_coverage(coverage, input.color);
        }
        return input.color * coverage;
    }
    if (input.shape.w < 0) {
        // Its rectangle goes as far as it's soft on each side, which is three times the spread of the blur, as a browser's does:
        // what's under a blur that goes from one edge to the other is the area of a bell curve.
        float reach = -input.shape.w;
        float2 box = input.shape.xy - reach;
        float spread = reach / 3 * 1.41421356;
        if (input.shape.z <= 0) {
            // Square, so it's the bell curve each way, as exactly as a browser has it.
            float2 near = (input.local + box) / spread;
            float2 far = (input.local - box) / spread;
            float2 each = 0.5 * float2(erf(near.x) - erf(far.x), erf(near.y) - erf(far.y));
            return input.color * each.x * each.y;
        }
        float2 away = abs(input.local) - box + input.shape.z;
        float distance = length(max(away, 0)) + min(max(away.x, away.y), 0) - input.shape.z;
        return input.color * 0.5 * (1 - erf(distance / spread));
    }
    // Something that's been turned has edges that don't line up with pixels, however square it is.
    if (input.shape.z > 0 || input.shape.w > 0 || transformed > 0) {
        float2 inside = abs(input.local) - input.shape.xy + input.shape.z;
        float distance = length(max(inside, 0)) + min(max(inside.x, inside.y), 0) - input.shape.z;
        float coverage = saturate(0.5 - distance * sharpness);
        if (input.shape.w > 0) {
            coverage *= saturate(0.5 + (distance + input.shape.w) * sharpness);
        }
        return input.color * coverage;
    }
    return input.color;
}

// What's drawn, and how much of what's behind each part of it is covered, for blending with two sources: red, green and blue can
// each be covered by a different amount, as they are for text drawn as ClearType draws it.
struct Drawn {
    float4 color : SV_Target0;
    float4 cover : SV_Target1;
};

// A color, through what `filter` says, as a browser has it: on its colors before they're multiplied by how opaque it is.
float4 filter_color(float4 color) {
    if (filtered <= 0 || color.a <= 0) {
        return color;
    }
    float4 plain = float4(color.rgb / color.a, 1);
    float3 rgb = saturate(float3(dot(filter_red, plain), dot(filter_green, plain), dot(filter_blue, plain)));
    float alpha = color.a * filter_opacity;
    return float4(rgb * alpha, alpha);
}

Drawn drawn(float4 color) {
    color = filter_color(color);
    Drawn out_value;
    out_value.color = color;
    out_value.cover = color.aaaa;
    return out_value;
}

Drawn image_ps(Pixel input) {
    return drawn(image_color(input) * round_clips_cover(input));
}

Drawn ps(Pixel input) {
    // A glyph with a byte for each part of each pixel, of which this reads the middle one, with the others on either side.
    // A browser doesn't do this through a filter either.
    if (input.uv.x >= 0 && input.shape.z < 0 && transformed <= 0 && input.inset.y <= 0 && filtered <= 0) {
        int2 at = int2(floor(input.uv));
        float3 cover = float3(atlas.Load(int3(at.x - 1, at.y, 0)), atlas.Load(int3(at, 0)), atlas.Load(int3(at.x + 1, at.y, 0)));
        if (text_gamma > 0) {
            float3 rgb = input.color.a > 0 ? input.color.rgb / input.color.a : input.color.rgb;
            cover = float3(browser_part(cover.r, rgb.r), browser_part(cover.g, rgb.g), browser_part(cover.b, rgb.b));
        }
        cover *= round_clips_cover(input);
        Drawn out_value;
        out_value.color = float4(input.color.rgb * cover, input.color.a * max(cover.r, max(cover.g, cover.b)));
        out_value.cover = float4(cover * input.color.a, out_value.color.a);
        return out_value;
    }
    return drawn(color_of(input) * round_clips_cover(input));
}
