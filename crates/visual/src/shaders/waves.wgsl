// Ondes : six lignes lumineuses, chacune portée par une partie du spectre.

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / p.resolution;
    let aspect = p.resolution.x / p.resolution.y;
    let y = (uv.y * 2.0 - 1.0) / aspect * 1.6;
    // Lignes calmes sur les bords, amples au milieu.
    let envelope = sin(uv.x * PI);
    var col = vec3<f32>(0.0);
    for (var i = 0; i < 6; i++) {
        let fi = f32(i);
        let amp = 0.04 + 0.45 * band(i * 10 + 3);
        let freq = 1.5 + fi * 0.9;
        let phase = p.time * (0.7 + fi * 0.23) + fi * 1.7;
        let wave = sin(uv.x * freq * TAU + phase) * amp * envelope;
        let d = abs(y - wave + (fi - 2.5) * 0.03);
        let width = 0.003 + 0.006 * p.beat;
        col += palette(fi / 6.0 + p.time * 0.02) * width / max(d, 0.0015);
    }
    col *= 0.35 + 0.8 * p.level;
    return tonemap(col);
}
