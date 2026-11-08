// Nébuleuse : nuages de bruit qui se déforment avec les graves.

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = centered(pos.xy);
    let t = p.time * 0.12;
    let q = vec2<f32>(
        fbm(uv * 2.0 + vec2<f32>(t, -t)),
        fbm(uv * 2.0 + vec2<f32>(-t * 0.7, t * 0.5) + 3.1),
    );
    let n = fbm(uv * 2.5 + q * (1.5 + 2.5 * p.bass) + vec2<f32>(0.0, t));
    // Fond profond, couleurs vives dans les zones denses.
    let deep = vec3<f32>(0.03, 0.01, 0.08);
    let bright = palette(n * 0.9 + p.time * 0.02 + p.treble * 0.4 + 0.55);
    var col = mix(deep, bright, smoothstep(0.25, 0.85, n));
    col *= (0.3 + 1.8 * n * n) * (0.45 + 1.1 * p.level);
    // Éclat au centre sur les temps forts.
    col += vec3<f32>(0.9, 0.6, 1.0) * p.beat * 0.35 * smoothstep(0.8, 0.0, length(uv));
    col *= smoothstep(1.2, 0.2, length(uv));
    return tonemap(col);
}
