MDLite UI proposal icon set
20 original vector icons drawn for this proposal. Not yet integrated or Windows-tested.

24x24 viewBox, 1.5 unit rounded strokes. Main activity icons display at 20 DIP, toolbar icons at 16 DIP. Do not stretch aspect ratio. Use currentColor for normal/hover/selected/disabled/high-contrast colors.

Implementation recommendation: preserve these SVGs as source assets; use native Direct2D geometry or build-time rasterization at DPI-specific sizes. Do not add a browser runtime solely for SVG. Validate 100%, 125%, 150%, 200%, light/dark/high contrast.

Window caption buttons in this set illustrate the proposal only; prefer Windows/DWM-managed caption behavior in the application. Provide accessible names and tooltips separately.
