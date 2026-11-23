import { WAVE_PATH, WAVE_STROKE } from "./WhisprMark";

/** The tray icon, as scripts/build-brand-icons.ts draws src-tauri/resources/tray_*.png; uses currentColor like a macOS template image. */
type TrayIconState = "idle" | "recording" | "transcribing";

const wave = (stroke: string, transform?: string) => (
  <path
    d={WAVE_PATH}
    fill="none"
    stroke={stroke}
    strokeWidth={WAVE_STROKE}
    strokeLinecap="round"
    strokeLinejoin="round"
    transform={transform}
  />
);

/** What is cut out of the disc while Whispr is busy. */
const CUTOUT: Record<Exclude<TrayIconState, "idle">, React.ReactNode> = {
  // Listening: the wave.
  recording: wave("#000", "translate(32 32) scale(0.68) translate(-32 -32)"),
  // Writing: an ellipsis.
  transcribing: [19, 32, 45].map((x) => (
    <circle key={x} cx={x} cy="32" r="4.6" fill="#000" />
  )),
};

const TrayIcon = ({
  state,
  size,
}: {
  state: TrayIconState;
  size: number | string;
}) => {
  const mask = `tray-icon-${state}`;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      xmlns="http://www.w3.org/2000/svg"
    >
      {state === "idle" ? (
        wave("currentColor")
      ) : (
        <>
          <mask id={mask}>
            <rect width="64" height="64" fill="#fff" />
            {CUTOUT[state]}
          </mask>
          <circle
            cx="32"
            cy="32"
            r="29"
            fill="currentColor"
            mask={`url(#${mask})`}
          />
        </>
      )}
    </svg>
  );
};

export default TrayIcon;
