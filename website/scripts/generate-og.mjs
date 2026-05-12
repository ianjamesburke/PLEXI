// Renders public/og-default.svg → public/og-default.png at 1200x630.
// Run with `npm run og` whenever the SVG changes.
import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import sharp from 'sharp';

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, '..');
const svgPath = resolve(root, 'public/og-default.svg');
const pngPath = resolve(root, 'public/og-default.png');

try {
  const svg = await readFile(svgPath);
  const png = await sharp(svg, { density: 150 })
    .resize(1200, 630)
    .png()
    .toBuffer();
  await writeFile(pngPath, png);
  console.log(`Wrote ${pngPath} (${png.length} bytes)`);
} catch (err) {
  console.error(`generate-og failed: ${err.message}`);
  process.exit(1);
}
