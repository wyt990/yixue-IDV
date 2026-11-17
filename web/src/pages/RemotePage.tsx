import { Navigate, useSearchParams } from "react-router-dom";

/** 远程控制不再是独立页面，旧地址转入终端并打开会话。 */
export function RemotePage() {
  const [params] = useSearchParams();
  const next = new URLSearchParams();
  const mac = params.get("mac") || params.get("assist") || "";
  if (mac) {
    next.set("assist", mac);
    next.set("mode", params.get("mode") === "control" ? "control" : "view");
  }
  const lab = params.get("lab");
  if (lab) next.set("lab", lab);
  const q = next.toString();
  return <Navigate to={q ? `/clients?${q}` : "/clients"} replace />;
}
