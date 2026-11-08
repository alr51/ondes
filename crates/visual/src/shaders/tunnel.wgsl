// Tunnel : anneaux qui défilent, d'autant plus vite que la musique est forte.

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = centered(pos.xy);
    let r = max(length(uv), 0.001);
    let a = atan2(uv.y, uv.x);
    let z = 0.35 / r + p.time * 0.8;
    let u = a / TAU + 0.5;
    // Spectre réparti sur le tour, en miroir : pas de raccord à ±π.
    let m = abs(fract(u * 2.0) * 2.0 - 1.0);
    let s = spectrum(m);
    let rings = pow(0.5 + 0.5 * sin(z * 5.0), 4.0);
    let stripes = smoothstep(0.55, 1.0, 0.5 + 0.5 * sin(u * TAU * 12.0 + z * 0.5)) * s;
    var col = palette(z * 0.04 + m * 0.15 + p.mid * 0.3) * (rings * (0.3 + p.bass) + stripes * 1.6);
    col += palette(z * 0.1) * p.beat * 0.4 * rings;
    // Le fond du tunnel se perd dans le noir.
    col *= clamp(r * 1.8, 0.0, 1.0) * (0.5 + 0.8 * p.level);
    return tonemap(col);
}
