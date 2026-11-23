/**
 * The wordmark: "whispr" in rounded strokes, its "w" the mark's wave in the
 * brand color. Drawn letters, not a font, so it looks the same everywhere.
 */
const WhisprTextLogo = ({
  width,
  height,
  className,
}: {
  width?: number;
  height?: number;
  className?: string;
}) => {
  return (
    <svg
      width={width}
      height={height}
      className={className}
      viewBox="0 0 186 78"
      fill="none"
      strokeWidth="7"
      strokeLinecap="round"
      strokeLinejoin="round"
      xmlns="http://www.w3.org/2000/svg"
      role="img"
      aria-label="Whispr"
    >
      {/* w */}
      <path
        className="stroke-logo-primary"
        d="M6 28 C10 28 12 48 16.5 48 C21 48 22 33 26 33 C30 33 31 48 35.5 48 C40 48 42 28 46 28"
      />
      <g className="stroke-text">
        {/* h */}
        <path d="M59 10 V48 M59 38 C59 32 63.5 28 69 28 C74.5 28 79 32 79 38 V48" />
        {/* i */}
        <path d="M93 28 V48" />
        {/* s */}
        <path d="M121 32 C118.5 29.5 116 28 113 28 C108.5 28 106 30.5 106 33.5 C106 40.5 121.5 35.5 121.5 42.5 C121.5 45.5 119 48 114 48 C110.5 48 108 46.8 105.5 44" />
        {/* p */}
        <path d="M135 28 V68 M135 38 C135 32 139.5 28 145 28 C150.5 28 155 32 155 38 C155 44 150.5 48 145 48 C139.5 48 135 44 135 38" />
        {/* r */}
        <path d="M169 28 V48 M169 38 C169 32 173 28 179 28" />
      </g>
      {/* The dot of the i. */}
      <circle cx="93" cy="14" r="4.6" className="fill-text" />
    </svg>
  );
};

export default WhisprTextLogo;
