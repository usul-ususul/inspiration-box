/*
 * Liquid Glass WebGL Renderer
 * 通过 displacement map + chromatic aberration + specular 实现
 * 真正的光学折射效果（非 backdrop-filter 毛玻璃）
 */

let glassGL = null;
let glassProgram = null;
let glassTexture = null;
let glassCanvas = null;
let glassAnimating = false;
let glassUniforms = {};

const GLASS_VS = `
attribute vec2 a_pos;
varying vec2 v_uv;
void main() {
    v_uv = a_pos * 0.5 + 0.5;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
`;

const GLASS_FS = `
precision highp float;
uniform sampler2D u_bg;
uniform vec2 u_res;
uniform float u_time;
varying vec2 v_uv;

/* 圆角矩形 SDF → 边缘距离因子 */
float edgeFactor(vec2 uv) {
    vec2 p = (uv - 0.5) * 2.0;
    float aspect = u_res.x / max(u_res.y, 1.0);
    p.x /= aspect;
    float r = 0.7;
    vec2 q = abs(p) - vec2(1.0 - r);
    float sdf = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
    return smoothstep(-0.35, 0.05, sdf);
}

/* 8-tap 模糊采样 */
vec3 sampleBlurred(sampler2D tex, vec2 uv, float radius) {
    vec2 px = radius / u_res;
    vec3 sum = vec3(0.0);
    sum += texture2D(tex, uv + vec2(px.x, 0.0)).rgb;
    sum += texture2D(tex, uv - vec2(px.x, 0.0)).rgb;
    sum += texture2D(tex, uv + vec2(0.0, px.y)).rgb;
    sum += texture2D(tex, uv - vec2(0.0, px.y)).rgb;
    sum += texture2D(tex, uv + vec2(px.x, px.y)).rgb;
    sum += texture2D(tex, uv - vec2(px.x, px.y)).rgb;
    sum += texture2D(tex, uv + vec2(-px.x, px.y)).rgb;
    sum += texture2D(tex, uv - vec2(-px.x, px.y)).rgb;
    return sum * 0.125;
}

void main() {
    vec2 uv = vec2(v_uv.x, 1.0 - v_uv.y);
    float edge = edgeFactor(v_uv);

    /* ── 1. 折射位移 ──
       边缘处内容向中心弯折（透镜效应） */
    vec2 toCenter = 0.5 - v_uv;
    float dist = length(toCenter);
    if (dist > 0.001) toCenter /= dist;
    else toCenter = vec2(0.0);
    float refraction = 0.05;
    vec2 disp = toCenter * edge * refraction;

    /* ── 2. 色差 ──
       R/G/B 通道不同位移量，模拟棱镜分光 */
    float ca = edge * 0.014;
    vec3 col;
    col.r = texture2D(u_bg, uv + disp + toCenter * ca).r;
    col.g = texture2D(u_bg, uv + disp).g;
    col.b = texture2D(u_bg, uv + disp - toCenter * ca).b;

    /* ── 3. 磨砂模糊 ──
       边缘更强，中心较弱 */
    float blurRadius = 1.5 + edge * 2.5;
    vec3 blurred = sampleBlurred(u_bg, uv + disp, blurRadius);
    col = mix(col, blurred, 0.3);

    /* ── 4. 镜面高光 ──
       顶边亮（光从上方来），底边暗 */
    float topHL = smoothstep(0.82, 1.0, v_uv.y) * 0.22;
    float sideHL = smoothstep(0.92, 1.0, abs(v_uv.x - 0.5) * 2.0) * 0.10;
    float botShade = smoothstep(0.15, 0.0, v_uv.y) * 0.06;
    col += vec3(topHL + sideHL) - vec3(botShade);

    /* ── 5. 环境光扫描 ──
       缓慢流动的光带，模拟环境光变化 */
    float sweep = sin(v_uv.x * 3.14 + u_time * 0.5) * 0.5 + 0.5;
    sweep *= smoothstep(0.7, 1.0, 1.0 - edge) * 0.04;
    col += vec3(sweep);

    /* ── 6. 冷色调 ── */
    col = mix(col, col * vec3(0.97, 0.99, 1.04), 0.05);

    /* ── 7. 透明度 ── */
    float alpha = 0.85 + edge * 0.15;

    gl_FragColor = vec4(col, alpha);
}
`;

function initGlass(canvas) {
    const gl = canvas.getContext('webgl', { alpha: true, premultipliedAlpha: false, antialias: false });
    if (!gl) {
        console.warn('[glass] WebGL not available');
        return false;
    }
    glassGL = gl;
    glassCanvas = canvas;

    const vs = gl.createShader(gl.VERTEX_SHADER);
    gl.shaderSource(vs, GLASS_VS);
    gl.compileShader(vs);
    if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
        console.error('[glass] VS error:', gl.getShaderInfoLog(vs));
        return false;
    }

    const fs = gl.createShader(gl.FRAGMENT_SHADER);
    gl.shaderSource(fs, GLASS_FS);
    gl.compileShader(fs);
    if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
        console.error('[glass] FS error:', gl.getShaderInfoLog(fs));
        return false;
    }

    glassProgram = gl.createProgram();
    gl.attachShader(glassProgram, vs);
    gl.attachShader(glassProgram, fs);
    gl.linkProgram(glassProgram);
    if (!gl.getProgramParameter(glassProgram, gl.LINK_STATUS)) {
        console.error('[glass] Link error:', gl.getProgramInfoLog(glassProgram));
        return false;
    }

    /* 全屏四边形 */
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([
        -1, -1,  1, -1,  -1, 1,
        -1,  1,  1, -1,   1, 1,
    ]), gl.STATIC_DRAW);

    const posLoc = gl.getAttribLocation(glassProgram, 'a_pos');
    gl.enableVertexAttribArray(posLoc);
    gl.vertexAttribPointer(posLoc, 2, gl.FLOAT, false, 0, 0);

    /* 纹理 */
    glassTexture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, glassTexture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    /* 1×1 透明初始纹理 */
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([0, 0, 0, 0]));

    /* 缓存 uniform 位置 */
    glassUniforms.bg = gl.getUniformLocation(glassProgram, 'u_bg');
    glassUniforms.res = gl.getUniformLocation(glassProgram, 'u_res');
    glassUniforms.time = gl.getUniformLocation(glassProgram, 'u_time');

    return true;
}

function updateGlassFrame(base64Data, width, height) {
    if (!glassGL || !glassTexture) return;

    /* base64 → Uint8Array */
    const binary = atob(base64Data);
    const pixels = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
        pixels[i] = binary.charCodeAt(i);
    }

    /* canvas 尺寸同步 */
    if (glassCanvas.width !== width || glassCanvas.height !== height) {
        glassCanvas.width = width;
        glassCanvas.height = height;
    }

    const gl = glassGL;
    gl.bindTexture(gl.TEXTURE_2D, glassTexture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
}

function renderGlass(time) {
    if (!glassGL || !glassAnimating) return;

    const gl = glassGL;
    const canvas = glassCanvas;

    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);

    gl.useProgram(glassProgram);

    gl.uniform1i(glassUniforms.bg, 0);
    gl.uniform2f(glassUniforms.res, canvas.width, canvas.height);
    gl.uniform1f(glassUniforms.time, time * 0.001);

    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, glassTexture);

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

    gl.drawArrays(gl.TRIANGLES, 0, 6);

    requestAnimationFrame(renderGlass);
}

function startGlassRender(canvas) {
    if (!initGlass(canvas)) return false;
    glassAnimating = true;
    requestAnimationFrame(renderGlass);
    return true;
}

function stopGlassRender() {
    glassAnimating = false;
}
