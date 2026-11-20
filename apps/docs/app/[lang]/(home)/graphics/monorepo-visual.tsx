import { SiNextdotjs, SiNuxt, SiSvelte } from "@icons-pack/react-simple-icons";

const tasks = [
  { label: "apps/web", Logo: SiNextdotjs },
  { label: "apps/docs", Logo: SiSvelte },
  { label: "app/blog", Logo: SiNuxt },
];

export function MonorepoVisual() {
  return (
    <div
      aria-hidden="true"
      className="flex size-full items-center justify-center bg-background-200 px-6"
    >
      <div className="w-full max-w-[280px] overflow-hidden rounded-2xl bg-background-100 shadow-(--ds-shadow-border-small)">
        <div className="border-gray-300 border-b px-4 py-3 text-gray-900 text-label-13-mono">
          <span className="text-gray-600">$</span> turbo run build
        </div>
        <div className="flex flex-col gap-2.5 px-4 py-3.5 text-label-13-mono">
          {tasks.map(({ label, Logo }, index) => (
            <div className="flex items-center gap-2" key={label}>
              <span className="text-gray-600">
                {index === tasks.length - 1 ? "└─" : "├─"}
              </span>
              <Logo
                aria-hidden="true"
                className="text-gray-1000"
                color="currentColor"
                size={14}
              />
              <span className="flex-1 text-gray-900">{label}</span>
              <span className="text-gray-900">built</span>
            </div>
          ))}
        </div>
        <div className="flex items-center justify-between border-gray-300 border-t px-4 py-2.5 text-gray-900 text-label-13-mono">
          <span>3 tasks</span>
          <span>420ms</span>
        </div>
      </div>
    </div>
  );
}
