// Anneau : le spectre en couronne, qui respire avec les temps forts.

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = centered(pos.xy);
    let r = length(uv);
    // Symétrique gauche / droite : graves en haut, aigus en bas.
    let x = abs(atan2(uv.x, -uv.y)) / PI;
    let s = spectrum(x * 0.9);
    let base = 0.2 + 0.035 * p.beat + 0.03 * p.bass;
    let outer = base + 0.22 * s;
    let hue = palette(x * 0.5 + p.time * 0.03);

    var col = vec3<f32>(0.0);
    // Barres pleines entre le cercle de base et le spectre.
    let inside = smoothstep(base - 0.004, base, r) * smoothstep(outer + 0.004, outer, r);
    col += hue * inside * (0.35 + 0.9 * s);
    // Halo autour du bord extérieur.
    col += hue * s * 0.012 / max(abs(r - outer), 0.003);
    // Disque central et fond.
    col += palette(p.time * 0.02 + 0.5) * 0.12 * smoothstep(base, 0.0, r) * (0.4 + p.level);
    col += hue * 0.04 * (1.0 - smoothstep(0.0, 0.9, r)) * (0.5 + p.beat);
    return tonemap(col);
}
