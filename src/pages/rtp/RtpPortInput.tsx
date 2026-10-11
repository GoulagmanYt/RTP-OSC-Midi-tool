import { useState } from "react";
import { Input } from "../../components/ui/Input";

type Props = {
  id: string;
  value?: number;
  onCommit: (port: number) => void;
};

// Remount the draft when an import, reset or rollback replaces the saved port.
export function RtpPortInput(props: Props) {
  return <PortDraft key={props.value ?? "empty"} {...props} />;
}

function PortDraft({ id, value, onCommit }: Props) {
  const [draft, setDraft] = useState(value?.toString() ?? "");
  return (
    <Input
      id={id}
      type="number"
      min={1024}
      max={65534}
      step={1}
      placeholder="5004"
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={() => {
        const port = Number(draft);
        if (draft.trim() !== "" && Number.isInteger(port) && port >= 1024 && port <= 65534) {
          setDraft(String(port));
          if (port !== value) onCommit(port);
        } else {
          setDraft(value?.toString() ?? "");
        }
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          event.currentTarget.blur();
        }
      }}
    />
  );
}
