// Rehype plugin: wrap every "YAMLRocks" prose mention in a branded span, so CSS
// can render it with the hero gradient and a leading rock emoji. Runs at build
// time over the page HTML (HAST). Code, pre, script, and style elements are
// skipped so identifiers and examples are left untouched.

const BRAND = "YAMLRocks";
const SKIP_TAGS = new Set(["code", "pre", "script", "style"]);

function hasBrandClass(node) {
  const cn = node.properties && node.properties.className;
  if (!cn) return false;
  return Array.isArray(cn) ? cn.includes("yamlrocks") : cn === "yamlrocks";
}

function brandSpan() {
  return {
    type: "element",
    tagName: "span",
    properties: { className: ["yamlrocks"] },
    children: [{ type: "text", value: BRAND }],
  };
}

export default function rehypeYamlrocks() {
  const walk = (node) => {
    if (!node.children || node.children.length === 0) return;
    if (node.tagName && SKIP_TAGS.has(node.tagName)) return;
    if (node.tagName === "span" && hasBrandClass(node)) return;

    const out = [];
    for (const child of node.children) {
      if (child.type === "text" && child.value.includes(BRAND)) {
        const parts = child.value.split(BRAND);
        parts.forEach((part, i) => {
          if (part) out.push({ type: "text", value: part });
          if (i < parts.length - 1) out.push(brandSpan());
        });
      } else {
        walk(child);
        out.push(child);
      }
    }
    node.children = out;
  };

  return (tree) => walk(tree);
}
