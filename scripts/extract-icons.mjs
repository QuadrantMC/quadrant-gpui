// Writes the Material Design icons the desktop app uses (react-icons/md) as
// SVG files under assets/icons, so the GPUI app draws the same glyphs.
//   node scripts/extract-icons.mjs [path-to-quadrant-checkout]
// Needs `bun install` to have run in the quadrant checkout.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const quadrant = resolve(process.argv[2] ?? join(here, "..", "..", "quadrant"));
const out = join(here, "..", "assets", "icons");

// Asset file name -> react-icons export. Keep in sync with `MdIcon` in src/assets.rs.
const icons = {
  "account-circle": "MdAccountCircle",
  add: "MdAdd",
  archive: "MdArchive",
  "arrow-back": "MdArrowBack",
  check: "MdCheck",
  clear: "MdClear",
  close: "MdClose",
  computer: "MdComputer",
  delete: "MdDelete",
  description: "MdDescription",
  edit: "MdEdit",
  folder: "MdFolder",
  info: "MdInfo",
  minimize: "MdMinimize",
  notifications: "MdNotifications",
  refresh: "MdRefresh",
  search: "MdSearch",
  settings: "MdSettings",
  share: "MdShare",
  sync: "MdSync",
};

const source = readFileSync(join(quadrant, "node_modules", "react-icons", "md", "index.mjs"), "utf8");

const toSvg = ({ tag, attr, child }) => {
  const attrs = Object.entries(attr)
    .map(([key, value]) => ` ${key}="${value}"`)
    .join("");
  const inner = child.map(toSvg).join("");
  return `<${tag}${attrs}${tag === "svg" ? ' xmlns="http://www.w3.org/2000/svg"' : ""}>${inner}</${tag}>`;
};

mkdirSync(out, { recursive: true });
for (const [file, name] of Object.entries(icons)) {
  const match = source.match(new RegExp(`export function ${name} \\(props\\) \\{\\s*return GenIcon\\((.*)\\)\\(props\\);`));
  if (!match) throw new Error(`${name} not found in react-icons/md`);
  writeFileSync(join(out, `${file}.svg`), toSvg(JSON.parse(match[1])) + "\n");
  console.log(`${file}.svg <- ${name}`);
}
