# PortPilot README Media Rules

These assets are used directly on the GitHub repository homepage.

## Export Rules

- `hero-banner.svg` stays SVG and is only used for the top hero image.
- The hero is a release-page visual, not a UI collage.
- The hero must have one dominant product window on the right and one brand/CTA column on the left.
- The hero must not use equal-weight floating cards or placeholder-looking blocks.
- Keep hero copy to one headline, up to two subtitle lines, and at most two CTAs.
- `dashboard-preview.png`, `actions-preview.png`, and `observability-preview.png` must be exported as wide PNGs.
- Never export the detail previews as square thumbnails.
- Preferred output size for detail previews: `1280x760`.
- Long copy inside cards must be manually wrapped before export.
- Buttons, pills, and status labels must be centered from actual text width, not estimated width.
- Keep a visible safe margin on all sides so GitHub scaling does not crop or crowd text.

## Validation Checklist

- Confirm preview PNGs are width-first, not square.
- Open the final exported PNGs locally before committing.
- Check that the right-side rail is fully visible.
- Check that no heading or card text is clipped.
- Check that the artwork still reads well at GitHub README width.
- Check that the hero has a single focal point and still reads like a product launch visual at README width.

## Export Notes

Use `sips` to preserve the original SVG aspect ratio when exporting:

```bash
sips -s format png dashboard-preview.svg --out dashboard-preview.png
```

Do not use Quick Look thumbnail export for production README images because it can produce square-cropped outputs.
