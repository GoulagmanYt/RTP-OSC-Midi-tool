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
    remotes.find((remote) => remote.name === session.name) ??
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
  return remotes.some((remote) => {
    if (remote.port !== session.port) return false;
    if (remote.name && remote.name === session.name) return true;
    if (remote.host === session.host) return true;
    return session.addresses.includes(remote.host);
  });
}

export function buildRemoteConnectionChecker(participants: RtpParticipantInfo[]) {
  const connectedAddrs = new Set(participants.map((participant) => participant.addr));
  return (remote: RtpRemoteEntry) => {
    const addr = `${remote.host}:${remote.port}`;
    if (connectedAddrs.has(addr)) return true;
    if (!remote.name) return false;
    return participants.some((participant) => participant.name === remote.name);
  };
}
