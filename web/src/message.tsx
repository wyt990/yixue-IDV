import {
  createContext,
  ReactNode,
  useCallback,
  useContext,
  useMemo,
  useState,
} from "react";

export type MessageKind = "success" | "warning" | "error";

type MessageItem = {
  id: number;
  kind: MessageKind;
  text: string;
};

type MessageApi = {
  success: (text: string) => void;
  warning: (text: string) => void;
  error: (text: string) => void;
  show: (kind: MessageKind, text: string) => void;
};

const MessageContext = createContext<MessageApi | null>(null);

let seq = 1;

export function MessageProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<MessageItem[]>([]);

  const dismiss = useCallback((id: number) => {
    setItems((prev) => prev.filter((m) => m.id !== id));
  }, []);

  const show = useCallback(
    (kind: MessageKind, text: string) => {
      const id = seq++;
      setItems((prev) => [...prev, { id, kind, text }]);
      window.setTimeout(() => dismiss(id), 3000);
    },
    [dismiss],
  );

  const api = useMemo<MessageApi>(
    () => ({
      show,
      success: (text) => show("success", text),
      warning: (text) => show("warning", text),
      error: (text) => show("error", text),
    }),
    [show],
  );

  return (
    <MessageContext.Provider value={api}>
      {children}
      <div className="msg-host" aria-live="polite">
        {items.map((m) => (
          <div key={m.id} className={`msg-toast msg-${m.kind}`} role="status">
            {m.text}
          </div>
        ))}
      </div>
    </MessageContext.Provider>
  );
}

export function useMessage(): MessageApi {
  const ctx = useContext(MessageContext);
  if (!ctx) {
    throw new Error("useMessage must be used within MessageProvider");
  }
  return ctx;
}
