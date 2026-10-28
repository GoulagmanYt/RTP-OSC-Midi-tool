import { PortInput } from "../../components/ui/PortInput";

type Props = {
  id: string;
  value?: number;
  resetNonce?: number;
  onCommit: (port: number) => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
};

export function RtpPortInput(props: Props) {
  return <PortInput {...props} min={1024} max={65534} placeholder="5004" />;
}
