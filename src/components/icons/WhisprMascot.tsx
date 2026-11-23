/**
 * Whispr's mascot: a round little listener whose mouth is the mark's wave,
 * one hand up for a high five. The outline and face follow the theme
 * (`mascot-line`).
 */
const WhisprMascot = ({ className = "" }: { className?: string }) => {
  return (
    <svg
      viewBox="0 0 151 165"
      className={className}
      fill="none"
      strokeLinecap="round"
      strokeLinejoin="round"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
    >
      {/* The raised hand, behind the body. */}
      <ellipse
        cx="123"
        cy="50"
        rx="15"
        ry="19"
        transform="rotate(24 123 50)"
        className="fill-logo-primary stroke-mascot-line"
        strokeWidth="5"
      />
      {/* The body. */}
      <path
        d="M72 36 C106 36 131 61 131 95 C131 130 106 153 72 153 C38 153 14 130 14 95 C14 61 38 36 72 36 Z"
        className="fill-logo-primary stroke-mascot-line"
        strokeWidth="5"
      />
      {/* A highlight on its head. */}
      <path
        d="M36 76 C41 63 51 54 64 50"
        stroke="#fff"
        strokeOpacity="0.55"
        strokeWidth="6"
      />
      {/* Eyes. */}
      <ellipse cx="53" cy="92" rx="6" ry="8.5" className="fill-mascot-line" />
      <ellipse cx="93" cy="92" rx="6" ry="8.5" className="fill-mascot-line" />
      {/* The mouth: the mark's wave. */}
      <path
        d="M57 113 C59.5 113 61 124 64.5 124 C68 124 69.5 116.5 73 116.5 C76.5 116.5 78 124 81.5 124 C85 124 86.5 113 89 113"
        className="stroke-mascot-line"
        strokeWidth="5"
      />
      {/* Cheeks. */}
      <circle cx="38" cy="110" r="7" fill="#fff" fillOpacity="0.35" />
      <circle cx="108" cy="110" r="7" fill="#fff" fillOpacity="0.35" />
    </svg>
  );
};

export default WhisprMascot;
