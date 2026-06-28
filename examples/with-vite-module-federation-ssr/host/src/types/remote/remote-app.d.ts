import type { ComponentType } from "react";

declare module "remote/remote-app" {
  const RemoteApp: ComponentType;
  export default RemoteApp;
}
