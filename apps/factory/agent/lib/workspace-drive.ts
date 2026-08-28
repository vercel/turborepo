export const WORKSPACE_DRIVE_MOUNT_PATH = "/factory/persist";
export const WORKSPACE_DRIVE_CHECKOUT_PATH = `${WORKSPACE_DRIVE_MOUNT_PATH}/workspace`;
export const WORKSPACE_DRIVE_FX_PATH = `${WORKSPACE_DRIVE_MOUNT_PATH}/fx`;

const WORKSPACE_CHECKOUT_PATH = "/factory/turborepo";
const FX_HOME_PATH = "/home/vercel/.fx";
const INITIALIZED_PATH = `${WORKSPACE_DRIVE_MOUNT_PATH}/.factory-initialized`;

export function isWorkspaceDriveEnabled(
  value = process.env.FACTORY_WORKSPACE_DRIVES
): boolean {
  return value === "1";
}

export function workspaceDriveName(sessionId: string): string {
  return `factory-eve-${sessionId}-drive`;
}

export function workspaceCheckoutRefreshScript(
  checkoutPath = WORKSPACE_CHECKOUT_PATH
): string {
  return `git -C ${checkoutPath} fetch --depth=1 --force origin main
git -C ${checkoutPath} reset --hard FETCH_HEAD`;
}

export function workspaceDriveInitializationScript(): string {
  return `set -eu
mkdir -p ${WORKSPACE_DRIVE_CHECKOUT_PATH} ${WORKSPACE_DRIVE_FX_PATH}
if [ ! -f ${INITIALIZED_PATH} ]; then
  cp -a ${WORKSPACE_CHECKOUT_PATH}/. ${WORKSPACE_DRIVE_CHECKOUT_PATH}/
  ${workspaceCheckoutRefreshScript(WORKSPACE_DRIVE_CHECKOUT_PATH)}
  if [ -d ${FX_HOME_PATH} ]; then
    cp -a ${FX_HOME_PATH}/. ${WORKSPACE_DRIVE_FX_PATH}/
  fi
  touch ${INITIALIZED_PATH}
fi
rm -rf ${WORKSPACE_CHECKOUT_PATH} ${FX_HOME_PATH}
ln -s ${WORKSPACE_DRIVE_CHECKOUT_PATH} ${WORKSPACE_CHECKOUT_PATH}
ln -s ${WORKSPACE_DRIVE_FX_PATH} ${FX_HOME_PATH}`;
}
