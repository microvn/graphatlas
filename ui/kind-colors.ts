// Shared kind→hue map so the canvas legend, sidebar tree, and any
// future place that surfaces a symbol kind all paint the same colour
// for the same kind. Hues live in the cool Tokyo Night band — see
// DESIGN.md "Graph node (canvas)" for the rationale on the per-kind
// breakdown vs the original single-`--accent-cool` collapse.

export const KIND_COLOR: Record<string, string> = {
  function: "#22C55E",
  method: "#A3A3A3",
  struct: "#7DD3FC",
  class: "#A78BFA",
  interface: "#5EEAD4",
  enum: "#FBBF24",
  trait: "#F472B6",
  module: "#737373",
  other: "#737373",
};

export const DEFAULT_KIND_COLOR = "#737373";

export const kindColor = (kind: string): string =>
  KIND_COLOR[kind.toLowerCase()] ?? DEFAULT_KIND_COLOR;
