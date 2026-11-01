// Okhsl, Björn Ottosson's hue, saturation and lightness over OKLab: its hue is OKLab's, its lightness is OKLab's with a
// toe that spaces the darks as eyes do, and its saturation is a share of the most chroma sRGB holds at that hue and
// lightness, so every hue reaches the gamut's edge at 1 and none is clipped. Ported from his reference implementation
// (https://bottosson.github.io/posts/colorpicker/), MIT licensed.

type Vec = [number, number, number];

/** Linear sRGB from OKLab. */
export function oklabToLinear(l: number, a: number, b: number): Vec {
  const [x, y, z] = [
    l + 0.3963377774 * a + 0.2158037573 * b,
    l - 0.1055613458 * a - 0.0638541728 * b,
    l - 0.0894841775 * a - 1.291485548 * b,
  ].map((value) => value ** 3) as Vec;
  return [
    4.0767416621 * x - 3.3077115913 * y + 0.2309699292 * z,
    -1.2684380046 * x + 2.6097574011 * y - 0.3413193965 * z,
    -0.0041960863 * x - 0.7034186147 * y + 1.707614701 * z,
  ];
}

/** sRGB's transfer function, from a linear channel. */
function encode(channel: number): number {
  return channel <= 0.0031308 ? 12.92 * channel : 1.055 * channel ** (1 / 2.4) - 0.055;
}

const K1 = 0.206;
const K2 = 0.03;
const K3 = (1 + K1) / (1 + K2);

/** Okhsl's lightness from OKLab's. */
export function toe(x: number): number {
  return 0.5 * (K3 * x - K1 + Math.sqrt((K3 * x - K1) ** 2 + 4 * K2 * K3 * x));
}

/** OKLab's lightness from Okhsl's. */
function toeInverse(x: number): number {
  return (x * x + K1 * x) / (K3 * (x + K2));
}

/** The most saturation, chroma over lightness, sRGB holds along the unit hue `a`, `b`. */
function maxSaturation(a: number, b: number): number {
  let k: [number, number, number, number, number];
  let w: Vec;
  if (-1.88170328 * a - 0.80936493 * b > 1) {
    k = [1.19086277, 1.76576728, 0.59662641, 0.75515197, 0.56771245];
    w = [4.0767416621, -3.3077115913, 0.2309699292];
  } else if (1.81444104 * a - 1.19445276 * b > 1) {
    k = [0.73956515, -0.45954404, 0.08285427, 0.1254107, 0.14503204];
    w = [-1.2684380046, 2.6097574011, -0.3413193965];
  } else {
    k = [1.35733652, -0.00915799, -1.1513021, -0.50559606, 0.00692167];
    w = [-0.0041960863, -0.7034186147, 1.707614701];
  }
  let s = k[0] + k[1] * a + k[2] * b + k[3] * a * a + k[4] * a * b;
  // One step of Halley's method, as the reference takes.
  const kl = 0.3963377774 * a + 0.2158037573 * b;
  const km = -0.1055613458 * a - 0.0638541728 * b;
  const ks = -0.0894841775 * a - 1.291485548 * b;
  const [l_, m_, s_] = [1 + s * kl, 1 + s * km, 1 + s * ks];
  const f = w[0] * l_ ** 3 + w[1] * m_ ** 3 + w[2] * s_ ** 3;
  const f1 = w[0] * 3 * kl * l_ ** 2 + w[1] * 3 * km * m_ ** 2 + w[2] * 3 * ks * s_ ** 2;
  const f2 = w[0] * 6 * kl * kl * l_ + w[1] * 6 * km * km * m_ + w[2] * 6 * ks * ks * s_;
  s -= (f * f1) / (f1 * f1 - 0.5 * f * f2);
  return s;
}

/** The lightness and chroma of the most colourful sRGB colour along the unit hue `a`, `b`. */
function cusp(a: number, b: number): [number, number] {
  const s = maxSaturation(a, b);
  const rgb = oklabToLinear(1, s * a, s * b);
  const l = Math.cbrt(1 / Math.max(...rgb));
  return [l, l * s];
}

/** How far along the line from (`l0`, 0) to (`l1`, `c1`) at the unit hue `a`, `b` the sRGB gamut's edge is. */
function gamutIntersection(a: number, b: number, l1: number, c1: number, l0: number, top: [number, number]): number {
  const [lc, cc] = top;
  if ((l1 - l0) * cc - (lc - l0) * c1 <= 0) return (cc * l0) / (c1 * lc + cc * (l0 - l1));
  let t = (cc * (l0 - 1)) / (c1 * (lc - 1) + cc * (l0 - l1));
  // One step of Halley's method towards the upper edge, as the reference takes.
  const dl = l1 - l0;
  const kl = 0.3963377774 * a + 0.2158037573 * b;
  const km = -0.1055613458 * a - 0.0638541728 * b;
  const ks = -0.0894841775 * a - 1.291485548 * b;
  const [ldt, mdt, sdt] = [dl + c1 * kl, dl + c1 * km, dl + c1 * ks];
  const l = l0 * (1 - t) + t * l1;
  const c = t * c1;
  const [l_, m_, s_] = [l + c * kl, l + c * km, l + c * ks];
  const cubes: Vec = [l_ ** 3, m_ ** 3, s_ ** 3];
  const firsts: Vec = [3 * ldt * l_ ** 2, 3 * mdt * m_ ** 2, 3 * sdt * s_ ** 2];
  const seconds: Vec = [6 * ldt * ldt * l_, 6 * mdt * mdt * m_, 6 * sdt * sdt * s_];
  const rows: Vec[] = [
    [4.0767416621, -3.3077115913, 0.2309699292],
    [-1.2684380046, 2.6097574011, -0.3413193965],
    [-0.0041960863, -0.7034186147, 1.707614701],
  ];
  const dot = (row: Vec, v: Vec) => row[0] * v[0] + row[1] * v[1] + row[2] * v[2];
  const steps = rows.map((row) => {
    const f = dot(row, cubes) - 1;
    const f1 = dot(row, firsts);
    const f2 = dot(row, seconds);
    const u = f1 / (f1 * f1 - 0.5 * f * f2);
    return u >= 0 ? -f * u : Number.MAX_VALUE;
  });
  t += Math.min(...steps);
  return t;
}

/** Okhsl's three chromas at a lightness and unit hue: where saturation 0.8 and 1 fall, and the curve's start. */
function chromas(l: number, a: number, b: number): Vec {
  const top = cusp(a, b);
  const max = gamutIntersection(a, b, l, 1, l, top);
  const [stS, stT] = [top[1] / top[0], top[1] / (1 - top[0])];
  const k = max / Math.min(l * stS, (1 - l) * stT);
  const sMid =
    0.11516993 +
    1 /
      (7.4477897 +
        4.1590124 * b +
        a *
          (-2.19557347 +
            1.75198401 * b +
            a * (-2.13704948 - 10.02301043 * b + a * (-4.24894561 + 5.38770819 * b + 4.69891013 * a))));
  const tMid =
    0.11239642 +
    1 /
      (1.6132032 -
        0.68124379 * b +
        a *
          (0.40370612 +
            0.90148123 * b +
            a * (-0.27087943 + 0.6122399 * b + a * (0.00299215 - 0.45399568 * b - 0.14661872 * a))));
  const [midA, midB] = [l * sMid, (1 - l) * tMid];
  const mid = 0.9 * k * Math.sqrt(Math.sqrt(1 / (1 / midA ** 4 + 1 / midB ** 4)));
  const [zeroA, zeroB] = [l * 0.4, (1 - l) * 0.8];
  const zero = Math.sqrt(1 / (1 / zeroA ** 2 + 1 / zeroB ** 2));
  return [zero, mid, max];
}

/** An Okhsl colour, its hue in degrees and its saturation and lightness from 0 to 1, as `#rrggbb`. */
export function okhsl(hue: number, saturation: number, lightness: number): string {
  if (lightness >= 1) return "#ffffff";
  if (lightness <= 0) return "#000000";
  const angle = (hue * Math.PI) / 180;
  const [a, b] = [Math.cos(angle), Math.sin(angle)];
  const l = toeInverse(lightness);
  const [zero, mid, max] = chromas(l, a, b);
  const MID = 0.8;
  let c: number;
  if (saturation < MID) {
    const t = saturation / MID;
    const k1 = MID * zero;
    const k2 = 1 - k1 / mid;
    c = (t * k1) / (1 - k2 * t);
  } else {
    const t = (saturation - MID) / (1 - MID);
    const k1 = ((1 - MID) * mid * mid) / (MID * MID * zero);
    const k2 = 1 - k1 / (max - mid);
    c = mid + (t * k1) / (1 - k2 * t);
  }
  return `#${oklabToLinear(l, c * a, c * b)
    .map((channel) =>
      Math.round(255 * encode(Math.min(1, Math.max(0, channel))))
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}
