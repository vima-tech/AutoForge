import React, { useId } from 'react';

const GRID = 5;
const CELL = 10;
const SIZE = GRID * CELL; // 50

// Deterministic PRNG seeded from a string (FNV-1a + mixing)
function makeRand(seed: string): () => number {
  let h = 0x811c9dc5;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return () => {
    h ^= h >>> 13;
    h = Math.imul(h, 0x9e3779b9) >>> 0;
    h ^= h >>> 7;
    h = Math.imul(h, 0x85ebca6b) >>> 0;
    h ^= h >>> 16;
    return (h >>> 0) / 0x100000000;
  };
}

// Derive a vivid, distinct color from seed. Only hue varies; sat/lit are fixed
// so every agent gets a clearly visible color regardless of theme.
function seedToColors(seed: string): [string, string] {
  // FNV-1a hash → mix → hue
  let h = 0x811c9dc5;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  h ^= h >>> 13;
  h = Math.imul(h, 0x9e3779b9) >>> 0;
  h ^= h >>> 7;
  h = Math.imul(h, 0x85ebca6b) >>> 0;
  const hue = (h >>> 0) % 360;
  return [
    `hsl(${hue},68%,62%)`,
    // Fully opaque dark tint of the same hue — no transparency, so the avatar
    // renders cleanly when overlaid on other surfaces.
    `hsl(${hue},34%,16%)`,
  ];
}

function generatePath(seed: string): string {
  const rand = makeRand(seed);
  const filled = new Array<boolean>(GRID * GRID).fill(false);

  for (let row = 0; row < GRID; row++) {
    for (let col = 0; col < 3; col++) {
      const on = rand() > 0.38;
      filled[row * GRID + col] = on;
      filled[row * GRID + (GRID - 1 - col)] = on; // mirror
    }
  }

  let d = '';
  for (let row = 0; row < GRID; row++) {
    for (let col = 0; col < GRID; col++) {
      if (filled[row * GRID + col]) {
        d += `M${col * CELL},${row * CELL}h${CELL}v${CELL}h-${CELL}z`;
      }
    }
  }
  return d;
}

interface PixelAvatarProps {
  seed: string;
  size: number;
}

export function PixelAvatar({ seed, size }: PixelAvatarProps) {
  const rx = Math.round(SIZE * 0.32); // = 16, in SVG coordinate space
  const d = generatePath(seed);
  const [fill, bg] = seedToColors(seed);
  const uid = useId();
  const clipId = `pxclip${uid.replace(/:/g, '')}`;

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${SIZE} ${SIZE}`}
      style={{ display: 'block', flexShrink: 0 }}
    >
      <defs>
        <clipPath id={clipId}>
          <rect width={SIZE} height={SIZE} rx={rx} ry={rx} />
        </clipPath>
      </defs>
      <g clipPath={`url(#${clipId})`}>
        <rect width={SIZE} height={SIZE} fill={bg} />
        <path d={d} fill={fill} />
      </g>
    </svg>
  );
}
