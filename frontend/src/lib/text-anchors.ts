export interface TextAnchor {
  text: string;
  startLocation: number;
  endLocation: number;
  range: Range;
}

export function createTextAnchors(root: Node, chapterStart: number): TextAnchor[] {
  const ownerDocument = root.ownerDocument ?? document;
  const walker = ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const anchors: TextAnchor[] = [];
  let normalizedOffset = 0;
  let textNode = walker.nextNode();

  while (textNode) {
    const value = textNode.nodeValue ?? '';
    for (const match of value.matchAll(/[^\p{White_Space}]+/gu)) {
      if (anchors.length > 0) normalizedOffset += 1;
      const text = match[0];
      const startLocation = chapterStart + normalizedOffset;
      normalizedOffset += Array.from(text).length;
      const range = ownerDocument.createRange();
      range.setStart(textNode, match.index);
      range.setEnd(textNode, match.index + text.length);
      anchors.push({
        text,
        startLocation,
        endLocation: chapterStart + normalizedOffset,
        range,
      });
    }
    textNode = walker.nextNode();
  }

  return anchors;
}

export function lastFullyVisibleLocation(
  anchors: TextAnchor[],
  viewport: Pick<DOMRectReadOnly, 'top' | 'right' | 'bottom' | 'left'>,
  chapterStart: number,
  chapterEnd: number,
): number {
  let location = chapterStart;
  let lastVisible: TextAnchor | null = null;

  for (const anchor of anchors) {
    const rectangles = Array.from(anchor.range.getClientRects()).filter(
      (rectangle) => rectangle.width > 0 || rectangle.height > 0,
    );
    if (rectangles.length === 0) continue;
    if (rectangles.every((rectangle) =>
      rectangle.top >= viewport.top &&
      rectangle.right <= viewport.right &&
      rectangle.bottom <= viewport.bottom &&
      rectangle.left >= viewport.left
    )) {
      lastVisible = anchor;
      location = Math.min(chapterEnd, anchor.endLocation);
    }
  }

  return lastVisible === anchors.at(-1) ? chapterEnd : location;
}

export function nearestTextAnchor(
  anchors: TextAnchor[],
  location: number,
): TextAnchor | null {
  let nearest: TextAnchor | null = null;
  let nearestDistance = Number.POSITIVE_INFINITY;

  for (const anchor of anchors) {
    const distance = location < anchor.startLocation
      ? anchor.startLocation - location
      : location > anchor.endLocation
        ? location - anchor.endLocation
        : 0;
    if (distance < nearestDistance) {
      nearest = anchor;
      nearestDistance = distance;
    }
  }

  return nearest;
}
