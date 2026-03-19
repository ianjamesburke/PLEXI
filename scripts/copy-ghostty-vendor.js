const fs = require("fs");
const path = require("path");

const repoRoot = path.resolve(__dirname, "..");
const packageRoot = path.join(repoRoot, "node_modules", "ghostty-web");
const distRoot = path.join(packageRoot, "dist");
const vendorRoot = path.join(repoRoot, "src", "mainview", "vendor", "ghostty");

const bundlePatches = [
  {
    filename: "ghostty-web.js",
    replacements: [
      {
        from: "Math.ceil(g.width)",
        to: "g.width",
        description: "preserve fractional glyph width for canvas cell metrics",
      },
      {
        from: "const QA = 2, BA = 1, gA = 15, EA = 100;",
        to: "const QA = 2, BA = 1, gA = 0, EA = 100;",
        description: "remove phantom DOM scrollbar reservation in FitAddon",
      },
    ],
  },
  {
    filename: "ghostty-web.umd.cjs",
    replacements: [
      {
        from: "Math.ceil(g.width)",
        to: "g.width",
        description: "preserve fractional glyph width for canvas cell metrics",
      },
      {
        from: "const EA=2,CA=1,IA=15,DA=100;",
        to: "const EA=2,CA=1,IA=0,DA=100;",
        description: "remove phantom DOM scrollbar reservation in FitAddon",
      },
    ],
  },
];

function copyFile(from, to) {
  fs.mkdirSync(path.dirname(to), { recursive: true });
  fs.copyFileSync(from, to);
}

function applyReplacement(content, replacement, filename) {
  if (content.includes(replacement.from)) {
    return {
      content: content.replace(replacement.from, replacement.to),
      status: "patched",
    };
  }
  if (content.includes(replacement.to)) {
    return {
      content,
      status: "already-fixed",
    };
  }
  throw new Error(
    `Expected to find either ${JSON.stringify(replacement.from)} or ${JSON.stringify(replacement.to)} in ${filename} while applying patch: ${replacement.description}`,
  );
}

function patchBundle(filename, replacements) {
  const filePath = path.join(vendorRoot, filename);
  let content = fs.readFileSync(filePath, "utf8");
  const statuses = [];
  for (const replacement of replacements) {
    const result = applyReplacement(content, replacement, filename);
    content = result.content;
    statuses.push(`${replacement.description}: ${result.status}`);
  }
  fs.writeFileSync(filePath, content);
  return statuses;
}

function main() {
  fs.mkdirSync(vendorRoot, { recursive: true });

  for (const name of fs.readdirSync(distRoot)) {
    const from = path.join(distRoot, name);
    if (fs.statSync(from).isFile()) {
      copyFile(from, path.join(vendorRoot, name));
    }
  }

  copyFile(path.join(packageRoot, "ghostty-vt.wasm"), path.join(vendorRoot, "ghostty-vt.wasm"));

  for (const bundle of bundlePatches) {
    const statuses = patchBundle(bundle.filename, bundle.replacements);
    console.log(`${bundle.filename}: ${statuses.join(", ")}`);
  }

  console.log("ghostty vendor assets copied and patched");
}

main();
