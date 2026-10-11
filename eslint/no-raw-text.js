/**
 * SPEC §13: user-visible text must go through t(). Flags literal text in Svelte markup and in
 * attributes that users see (title, aria-label, placeholder, alt). Text made only of digits,
 * punctuation and symbols is allowed.
 */
const VISIBLE_ATTRIBUTES = new Set(["title", "aria-label", "placeholder", "alt", "label"]);
const NOT_WORDS = /^[\s\d\p{P}\p{S}]*$/u;

/** @type {import("eslint").Rule.RuleModule} */
export default {
  meta: {
    type: "problem",
    docs: { description: "Disallow user-visible literal text in Svelte markup" },
    messages: { raw: "User-visible text must go through t() (SPEC §13): {{text}}" },
    schema: [],
  },
  create(context) {
    const check = (node, text) => {
      if (!NOT_WORDS.test(text)) {
        context.report({ node, messageId: "raw", data: { text: text.trim().slice(0, 40) } });
      }
    };
    return {
      SvelteText(node) {
        const parent = node.parent?.type;
        if (parent === "SvelteAttribute" || parent === "SvelteStyleElement" || parent === "SvelteScriptElement") return;
        check(node, node.value);
      },
      SvelteAttribute(node) {
        if (!VISIBLE_ATTRIBUTES.has(node.key.name)) return;
        for (const part of node.value) {
          if (part.type === "SvelteLiteral") check(part, part.value);
        }
      },
    };
  },
};
