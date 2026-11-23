import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type MeetingCaptureStatus } from "@/bindings";
import { notify } from "@/stores/noticeStore";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";
import { ToggleSwitch } from "../ui/ToggleSwitch";

interface MeetingCaptureProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

/** How often the status is read again while meeting capture is on. */
const REFRESH_MS = 2000;

const formatDuration = (seconds: number): string => {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = String(seconds % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}`
    : `${minutes}:${rest}`;
};

/**
 * Turns on the endpoint the browser extension sends meetings to, and shows
 * what the extension needs to connect: whether Handy is listening, and the
 * pairing token.
 */
export const MeetingCapture: React.FC<MeetingCaptureProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const [status, setStatus] = useState<MeetingCaptureStatus | null>(null);
  const [updating, setUpdating] = useState(false);
  const [copied, setCopied] = useState(false);
  const enabled = status?.enabled ?? false;

  useEffect(() => {
    let current = true;
    const refresh = () =>
      commands
        .getMeetingCaptureStatus()
        .then((next) => current && setStatus(next))
        .catch(() => {});
    refresh();
    // Only a meeting in progress or a failed start changes it from outside.
    const timer = enabled ? setInterval(refresh, REFRESH_MS) : undefined;
    return () => {
      current = false;
      clearInterval(timer);
    };
  }, [enabled]);

  const failed = useCallback(
    (error: string) =>
      notify({
        title: t("settings.advanced.meetingCapture.changeFailed"),
        detail: error,
        level: "error",
      }),
    [t],
  );

  const toggle = async (next: boolean) => {
    setUpdating(true);
    try {
      const result = await commands.setMeetingCaptureEnabled(next);
      if (result.status === "ok") {
        setStatus(result.data);
      } else {
        failed(result.error);
      }
    } catch (error) {
      failed(String(error));
    } finally {
      setUpdating(false);
    }
  };

  const regenerate = async () => {
    try {
      const result = await commands.regenerateMeetingCaptureToken();
      if (result.status === "ok") {
        setStatus(result.data);
      } else {
        failed(result.error);
      }
    } catch (error) {
      failed(String(error));
    }
  };

  const copyToken = async () => {
    if (!status?.token) return;
    try {
      await navigator.clipboard.writeText(status.token);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (error) {
      failed(String(error));
    }
  };

  const statusText = (current: MeetingCaptureStatus): string => {
    if (current.recording_seconds != null) {
      return t("settings.advanced.meetingCapture.status.recording", {
        duration: formatDuration(current.recording_seconds),
      });
    }
    if (current.listening) {
      return t("settings.advanced.meetingCapture.status.listening", {
        port: current.port,
      });
    }
    return t("settings.advanced.meetingCapture.status.failed", {
      error: current.error ?? "",
    });
  };

  return (
    <>
      <ToggleSwitch
        checked={enabled}
        onChange={toggle}
        isUpdating={updating}
        disabled={status === null}
        label={t("settings.advanced.meetingCapture.title")}
        description={t("settings.advanced.meetingCapture.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
      {status && enabled && (
        <>
          <SettingContainer
            title={t("settings.advanced.meetingCapture.status.title")}
            description={t(
              "settings.advanced.meetingCapture.status.description",
            )}
            descriptionMode={descriptionMode}
            grouped={grouped}
          >
            <span
              className={`text-xs ${status.listening ? "text-ink-2" : "text-error"}`}
            >
              {statusText(status)}
            </span>
          </SettingContainer>
          <SettingContainer
            title={t("settings.advanced.meetingCapture.token.title")}
            description={t(
              "settings.advanced.meetingCapture.token.description",
            )}
            descriptionMode={descriptionMode}
            grouped={grouped}
            layout="stacked"
          >
            <div className="flex items-center gap-2">
              <div className="flex-1 min-w-0 px-2 min-h-8 flex items-center bg-wash border border-line-strong rounded-md text-xs font-mono select-text">
                <span className="truncate">{status.token}</span>
              </div>
              <Button variant="secondary" size="sm" onClick={copyToken}>
                {copied
                  ? t("settings.advanced.meetingCapture.token.copied")
                  : t("common.copy")}
              </Button>
              <Button variant="outline" size="sm" onClick={regenerate}>
                {t("settings.advanced.meetingCapture.token.regenerate")}
              </Button>
            </div>
          </SettingContainer>
        </>
      )}
    </>
  );
};
