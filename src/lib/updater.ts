import { getVersion } from "@tauri-apps/api/app";
import type { Update } from "@tauri-apps/plugin-updater";

export type UpdaterPhase =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "restarting"
  | "upToDate"
  | "error";

export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string;
  notes?: string;
  pubDate?: string;
}

export interface UpdateProgressEvent {
  event: "Started" | "Progress" | "Finished";
  total?: number;
  downloaded?: number;
}

export interface UpdateHandle {
  version: string;
  notes?: string;
  date?: string;
  downloadAndInstall: (
    onProgress?: (event: UpdateProgressEvent) => void,
  ) => Promise<void>;
}

export async function getCurrentVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

function mapHandle(update: Update): UpdateHandle {
  return {
    version: (update as unknown as { version?: string }).version ?? "",
    notes: (update as unknown as { notes?: string }).notes,
    date: (update as unknown as { date?: string }).date,
    async downloadAndInstall(onProgress?: (event: UpdateProgressEvent) => void) {
      await (
        update as unknown as {
          downloadAndInstall: (
            cb?: (event: {
              event: "Started" | "Progress" | "Finished";
              data?: { contentLength?: number; chunkLength?: number };
            }) => void,
          ) => Promise<void>;
        }
      ).downloadAndInstall((event) => {
        if (!onProgress) return;
        if (event.event === "Started") {
          onProgress({
            event: "Started",
            total: event.data?.contentLength ?? 0,
            downloaded: 0,
          });
          return;
        }
        if (event.event === "Progress") {
          onProgress({
            event: "Progress",
            downloaded: event.data?.chunkLength ?? 0,
          });
          return;
        }
        onProgress({ event: "Finished" });
      });
    },
  };
}

export async function checkForUpdate(): Promise<
  | { status: "up-to-date" }
  | { status: "available"; info: UpdateInfo; update: UpdateHandle }
> {
  const { check } = await import("@tauri-apps/plugin-updater");
  const currentVersion = await getCurrentVersion();
  const update = await check({ timeout: 30000 } as never);
  if (!update) {
    return { status: "up-to-date" };
  }

  const mapped = mapHandle(update);
  return {
    status: "available",
    info: {
      currentVersion,
      availableVersion: mapped.version,
      notes: mapped.notes,
      pubDate: mapped.date,
    },
    update: mapped,
  };
}

export async function relaunchApp(): Promise<void> {
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
