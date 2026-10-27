uniform sampler2D backdrop;
uniform vec2 capture_origin;
uniform vec2 capture_size;
uniform float texture_size;
uniform vec2 center;
uniform float radius;
uniform float sigma;
uniform vec3 tint;
out vec4 frag_color;

void main() {
    vec2 delta = gl_FragCoord.xy - center;
    float distance_to_edge = length(delta) - radius;
    float coverage = 1.0 - smoothstep(-0.5, 0.5, distance_to_edge);
    if (coverage <= 0.0) discard;

    // A local Gaussian kernel: only the small patch under the nib is copied
    // and sampled. Clamp at window edges, never sample uninitialized texels.
    vec3 blurred = vec3(0.0);
    float total = 0.0;
    for (int y = -3; y <= 3; ++y) {
        for (int x = -3; x <= 3; ++x) {
            vec2 offset = vec2(float(x), float(y)) * 0.75;
            float weight = exp(-0.5 * dot(offset, offset));
            vec2 pixel = clamp(gl_FragCoord.xy - capture_origin + offset * sigma,
                               vec2(0.5), capture_size - 0.5);
            blurred += texture(backdrop, pixel / texture_size).rgb * weight;
            total += weight;
        }
    }
    vec3 glass = mix(blurred / total, tint, 0.12);
    // Quiet upper-left reflection, fixed to the material rather than animated.
    float light = clamp(dot(delta / radius, vec2(-0.45, 0.7)), -1.0, 1.0);
    glass = mix(glass, tint, 0.035 * (light + 1.0));
    frag_color = vec4(glass * coverage, coverage);
}
