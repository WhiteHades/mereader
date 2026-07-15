import { describe, expect, it } from 'vitest';
import {
  createTextAnchors,
  lastFullyVisibleLocation,
  nearestTextAnchor,
} from '../lib/text-anchors';

function rect(top: number, bottom: number): DOMRect {
  return {
    top,
    bottom,
    left: 10,
    right: 90,
    width: 80,
    height: bottom - top,
    x: 10,
    y: top,
    toJSON: () => ({}),
  };
}

function setRects(range: Range, ...rectangles: DOMRect[]): void {
  Object.defineProperty(range, 'getClientRects', {
    configurable: true,
    value: () => rectangles,
  });
}

describe('DOM text anchors', () => {
  it('maps Unicode tokens to normalized code-point offsets in DOM order', () => {
    const root = document.createElement('div');
    root.append(document.createTextNode('  Cafe\u0301\t'));
    const emphasis = document.createElement('em');
    emphasis.textContent = '\u03b2\u{1f600}';
    root.append(emphasis, document.createTextNode('\u0085tail  '));

    const anchors = createTextAnchors(root, 100);

    expect(anchors.map(({ text, startLocation, endLocation }) => ({
      text,
      startLocation,
      endLocation,
    }))).toEqual([
      { text: 'Cafe\u0301', startLocation: 100, endLocation: 105 },
      { text: '\u03b2\u{1f600}', startLocation: 106, endLocation: 108 },
      { text: 'tail', startLocation: 109, endLocation: 113 },
    ]);
    expect(anchors.map((anchor) => [anchor.range.startOffset, anchor.range.endOffset])).toEqual([
      [2, 7],
      [0, 3],
      [1, 5],
    ]);
  });

  it('returns the end of the last fully visible token and permits chapter end', () => {
    const root = document.createElement('p');
    root.textContent = 'first second final';
    const anchors = createTextAnchors(root, 0);
    const viewport = rect(0, 100);

    setRects(anchors[0].range, rect(-2, 12));
    setRects(anchors[1].range, rect(20, 40));
    setRects(anchors[2].range, rect(110, 130));
    expect(lastFullyVisibleLocation(anchors, viewport, 0, 18)).toBe(12);

    setRects(anchors[2].range, rect(60, 80));
    expect(lastFullyVisibleLocation(anchors, viewport, 0, 18)).toBe(18);
  });

  it('keeps chapter start when no text token is visible and finds the nearest anchor', () => {
    const root = document.createElement('p');
    root.textContent = 'alpha beta';
    const anchors = createTextAnchors(root, 50);
    const viewport = rect(0, 100);

    setRects(anchors[0].range, rect(500, 520));
    setRects(anchors[1].range, rect(540, 560));

    expect(lastFullyVisibleLocation(anchors, viewport, 50, 60)).toBe(50);
    expect(nearestTextAnchor(anchors, 55)).toBe(anchors[0]);
    expect(nearestTextAnchor(anchors, 56)).toBe(anchors[1]);
  });
});
