export function RemoteCacheVisual() {
  return (
    <div
      aria-hidden="true"
      className="flex size-full items-center justify-center bg-background-200 px-6"
    >
      <div className="flex flex-col gap-2 text-copy-14-mono leading-6 tracking-tight">
        <p className="text-gray-900">
          <span className="text-green-900">329 successful</span>, 329 total
        </p>
        <p className="text-gray-900">
          <span className="font-bold">329 cached</span>, 329 total
        </p>
        <p className="text-gray-900">
          42ms{" "}
          <span className="bg-gradient-to-r from-[#0096FF] to-[#FF1E56] bg-clip-text text-transparent">
            &gt;&gt;&gt; FULL TURBO
          </span>
        </p>
      </div>
    </div>
  );
}
