/** The mark's wave on a 64 x 64 canvas; scripts/build-brand-icons.ts draws the same one. */
export const WAVE_PATH =
  "M8 21 C12.5 21 15 43 20.5 43 C26 43 27 26.5 32 26.5 C37 26.5 38 43 43.5 43 C49 43 51.5 21 56 21";
export const WAVE_STROKE = 6.5;

/** Whispr's mark: a quiet sound wave that reads as a "w". Drawn in the text color. */
const WhisprMark = ({
  width,
  height,
}: {
  width?: number | string;
  height?: number | string;
  // The sidebar passes a className for its icons; the mark ignores it.
  className?: string;
}) => (
  <svg
    width={width || 64}
    height={height || 64}
    viewBox="0 0 64 64"
    className="fill-none stroke-text"
    strokeWidth={WAVE_STROKE}
    strokeLinecap="round"
    strokeLinejoin="round"
    xmlns="http://www.w3.org/2000/svg"
  >
    <path d={WAVE_PATH} />
  </svg>
);

export default WhisprMark;
