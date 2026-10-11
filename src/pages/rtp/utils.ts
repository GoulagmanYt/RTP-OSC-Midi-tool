import type { RtpParticipantInfo, RtpRemoteEntry, RtpSessionInfo } from "../../api-types";

export function syncAutoConnectRemotes(remotes: RtpRemoteEntry[], sessions: RtpSessionInfo[]) {
  let changed = false;
  const next = remotes.map((remote) => {
    if (!remote.autoConnect || !remote.name) return remote;
    const match = sessions.find((session) => session.name.toLowerCase() === remote.name.toLowerCase());
    if (!match) return remote;
    const host = match.addresses[0] ?? match.host;
    if (remote.host !== host || remote.port !== match.port) {
      changed = true;
      return { ...remote, host, port: match.port };
    }
    return remote;
  });
  return { changed, remotes: next };
}

export function mergeSessionIntoRemotes(
  remotes: RtpRemoteEntry[],
  session: RtpSessionInfo,
  createId: () => string
) {
  const host = session.addresses[0] ?? session.host;
  const existing =
    remotes.find((remote) => remote.name && remote.name.toLowerCase() === session.name.toLowerCase()) ??
    remotes.find((remote) => remote.host === host && remote.port === session.port);

  if (existing) {
    return remotes.map((remote) =>
      remote.id === existing.id
        ? { ...remote, name: session.name, host, port: session.port, autoConnect: true }
        : remote
    );
  }

  return [
    ...remotes,
    {
      id: createId(),
      name: session.name,
      host,
      port: session.port,
      autoConnect: true,
    },
  ];
}

export function isSelectedSession(remotes: RtpRemoteEntry[], session: RtpSessionInfo) {
  const sessionHost = normalizeHost(session.host);
  const sessionAddrs = session.addresses.map(normalizeHost);
  return remotes.some((remote) => {
    if (remote.port !== session.port) return false;
    if (remote.name && remote.name.toLowerCase() === session.name.toLowerCase()) return true;
    const remoteHost = normalizeHost(remote.host);
    if (remoteHost === sessionHost) return true;
    return sessionAddrs.includes(remoteHost);
  });
}

export function buildRemoteConnectionChecker(participants: RtpParticipantInfo[]) {
  const connectedAddrs = participants.flatMap((participant) => {
    const match = /^(\[[^\]]+\]|[^:]+):(\d+)$/.exec(participant.addr);
    return match ? [{ host: normalizeHost(match[1]), port: Number(match[2]) }] : [];
  });
  return (remote: RtpRemoteEntry) => {
    const host = normalizeHost(remote.host);
    if (connectedAddrs.some((addr) => addr.host === host && Math.abs(addr.port - remote.port) <= 1)) return true;
    if (!remote.name) return false;
    return participants.some((participant) => participant.name === remote.name);
  };
}

function normalizeHost(host: string) {
  const bare = host.replace(/^\[|\]$/g, "");
  if (!bare.includes(":")) return bare.toLowerCase();
  const [ip, scope] = bare.split("%");
  try {
    const canonical = new URL(`http://[${ip}]/`).hostname;
    return `${canonical}${scope ? `%${scope}` : ""}`;
  } catch {
    return bare.toLowerCase();
  }
}
