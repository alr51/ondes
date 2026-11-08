// Commun à tous les effets : paramètres, triangle plein écran, outils.

struct Params {
    resolution: vec2<f32>,
    // Secondes écoulées (avance plus vite quand la musique est forte).
    time: f32,
    // Niveau global, 0 à 1.
    level: f32,
    // Impulsion d'un temps fort : 1 au coup, puis retombe.
    beat: f32,
    bass: f32,
    mid: f32,
    treble: f32,
    // 64 bandes du spectre (log, 40 Hz à 16 kHz), 0 à 1.
    bands: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> p: Params;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;

fn band(i: i32) -> f32 {
    let j = clamp(i, 0, 63);
    return p.bands[j / 4][j % 4];
}

// Spectre interpolé, x de 0 (graves) à 1 (aigus).
fn spectrum(x: f32) -> f32 {
    let f = clamp(x, 0.0, 1.0) * 63.0;
    let i = i32(floor(f));
    return mix(band(i), band(i + 1), fract(f));
}

// Coordonnées centrées, hauteur de -0.5 à 0.5.
fn centered(pos: vec2<f32>) -> vec2<f32> {
    return (pos - 0.5 * p.resolution) / p.resolution.y;
}

fn hash(q: vec2<f32>) -> f32 {
    return fract(sin(dot(q, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(q: vec2<f32>) -> f32 {
    let i = floor(q);
    let f = fract(q);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y,
    );
}

fn fbm(q: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var x = q;
    for (var k = 0; k < 5; k++) {
        v += a * noise(x);
        x = x * 2.02 + vec2<f32>(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

// Palette cosinus (I. Quilez).
fn palette(t: f32) -> vec3<f32> {
    return 0.5 + 0.5 * cos(TAU * (t + vec3<f32>(0.0, 0.33, 0.67)));
}

// Compression douce des hautes lumières.
fn tonemap(c: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(1.0 - exp(-max(c, vec3<f32>(0.0)) * 1.4), 1.0);
}
