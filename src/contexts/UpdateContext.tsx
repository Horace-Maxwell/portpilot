import type { ComponentChildren } from "preact";
import { createContext } from "preact";
import { useCallback, useContext, useEffect, useRef, useState } from "preact/hooks";

import type {
  UpdateHandle,
  UpdateInfo,
  UpdateProgressEvent,
  UpdaterPhase,
} from "../lib/updater";
import { checkForUpdate, relaunchApp } from "../lib/updater";

interface UpdateContextValue {
  phase: UpdaterPhase;
  hasUpdate: boolean;
  updateInfo: UpdateInfo | null;
  updateHandle: UpdateHandle | null;
  isDismissed: boolean;
  progressTotal: number;
  progressDownloaded: number;
  error: string | null;
  dismissUpdate: () => void;
  resetDismiss: () => void;
  checkUpdate: () => Promise<boolean>;
  installUpdate: () => Promise<void>;
}

const UpdateContext = createContext<UpdateContextValue | undefined>(undefined);
const DISMISSED_KEY = "portpilot:update:dismissed-version";

export function UpdateProvider(props: { children: ComponentChildren }) {
  const [phase, setPhase] = useState<UpdaterPhase>("idle");
  const [hasUpdate, setHasUpdate] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [updateHandle, setUpdateHandle] = useState<UpdateHandle | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isDismissed, setIsDismissed] = useState(false);
  const [progressTotal, setProgressTotal] = useState(0);
  const [progressDownloaded, setProgressDownloaded] = useState(0);
  const checkingRef = useRef(false);

  useEffect(() => {
    if (!updateInfo?.availableVersion) return;
    setIsDismissed(localStorage.getItem(DISMISSED_KEY) === updateInfo.availableVersion);
  }, [updateInfo?.availableVersion]);

  const checkUpdate = useCallback(async () => {
    if (checkingRef.current) return false;

    checkingRef.current = true;
    setPhase("checking");
    setError(null);
    try {
      const result = await checkForUpdate();
      if (result.status === "available") {
        setHasUpdate(true);
        setUpdateInfo(result.info);
        setUpdateHandle(result.update);
        setPhase("available");
        setIsDismissed(localStorage.getItem(DISMISSED_KEY) === result.info.availableVersion);
        return true;
      }

      setHasUpdate(false);
      setUpdateInfo(null);
      setUpdateHandle(null);
      setPhase("upToDate");
      return false;
    } catch (reason) {
      setPhase("error");
      setError(reason instanceof Error ? reason.message : String(reason));
      return false;
    } finally {
      checkingRef.current = false;
    }
  }, []);

  const dismissUpdate = useCallback(() => {
    if (updateInfo?.availableVersion) {
      localStorage.setItem(DISMISSED_KEY, updateInfo.availableVersion);
      setIsDismissed(true);
    }
  }, [updateInfo?.availableVersion]);

  const resetDismiss = useCallback(() => {
    localStorage.removeItem(DISMISSED_KEY);
    setIsDismissed(false);
  }, []);

  const installUpdate = useCallback(async () => {
    if (!updateHandle) {
      throw new Error("No update is available right now.");
    }

    setPhase("downloading");
    setProgressDownloaded(0);
    setProgressTotal(0);
    setError(null);

    await updateHandle.downloadAndInstall((event: UpdateProgressEvent) => {
      if (event.event === "Started") {
        setProgressTotal(event.total ?? 0);
        setProgressDownloaded(0);
        return;
      }
      if (event.event === "Progress") {
        setProgressDownloaded((current) => current + (event.downloaded ?? 0));
        return;
      }
      setPhase("installing");
    });

    setPhase("restarting");
    await relaunchApp();
  }, [updateHandle]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void checkUpdate();
    }, 1200);
    return () => window.clearTimeout(timer);
  }, [checkUpdate]);

  const value: UpdateContextValue = {
    phase,
    hasUpdate,
    updateInfo,
    updateHandle,
    isDismissed,
    progressTotal,
    progressDownloaded,
    error,
    dismissUpdate,
    resetDismiss,
    checkUpdate,
    installUpdate,
  };

  return <UpdateContext.Provider value={value}>{props.children}</UpdateContext.Provider>;
}

export function useUpdate() {
  const context = useContext(UpdateContext);
  if (!context) {
    throw new Error("useUpdate must be used within UpdateProvider");
  }
  return context;
}
