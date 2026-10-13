// Windows stand-in for the fixture mirror's rustup, cargo, rustc, uv, Python,
// Go, and smoke-tool, the counterpart of its POSIX shell scripts. One native
// executable acts as the tool it is named after, as rustup's proxies do, and
// logs each call to <state>\calls.log. setup-fixture-mirror.mjs compiles it
// with the .NET Framework's csc.exe (C# 5) and replaces the state directory.
using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Text;
using Microsoft.Win32.SafeHandles;

static class Standin {
  const string State = @"@STATE@";
  const string Triple = "x86_64-pc-windows-msvc";

  [StructLayout(LayoutKind.Sequential)]
  struct Information {
    public uint Attributes, Created1, Created2, Accessed1, Accessed2, Written1, Written2;
    public uint Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow;
  }

  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool GetFileInformationByHandle(SafeFileHandle file, out Information information);

  static string Env(string name) { return Environment.GetEnvironmentVariable(name); }
  static void Say(string text) { Console.Out.Write(text + "\n"); }
  static string Json(string text) { return "\"" + text.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\""; }
  // The ordinary spelling, as the real tools print paths.
  static string Plain(string path) { return path.StartsWith(@"\\?\") ? path.Substring(4) : path; }

  // Like `test -ef`: the same file, through any hard link.
  static string Identity(string path) {
    try {
      using (var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete)) {
        Information information;
        if (!GetFileInformationByHandle(file.SafeFileHandle, out information)) return null;
        return information.Volume + ":" + information.IndexHigh + ":" + information.IndexLow;
      }
    } catch (Exception) {
      return null;
    }
  }

  // Append-only access writes each line at the end, like O_APPEND, so
  // concurrent tasks and the fixture pnpm never overwrite each other.
  static void Log(string line) {
    byte[] bytes = Encoding.UTF8.GetBytes(line + "\n");
    using (var log = new FileStream(Path.Combine(State, "calls.log"), FileMode.Append, FileSystemRights.AppendData,
        FileShare.ReadWrite | FileShare.Delete, 4096, FileOptions.None)) {
      log.Write(bytes, 0, bytes.Length);
    }
  }

  static int Main(string[] args) {
    string self = Process.GetCurrentProcess().MainModule.FileName;
    string name = Path.GetFileNameWithoutExtension(self).ToLowerInvariant();
    string all = string.Join(" ", args);
    string first = args.Length > 0 ? args[0] : "";
    string pair = args.Length > 1 ? first + " " + args[1] : all;
    // turbo reads the CPython builds from `uv python list`; it is not a call.
    if (name == "uv" && pair == "python list") {
      Say(File.ReadAllText(Path.Combine(State, "uv-python-list.json")));
      return 0;
    }
    Log(name + " " + all);
    string secret = " secret=" + (Env("SMOKE_SECRET") ?? "filtered");
    string root = Plain(Directory.GetCurrentDirectory());
    switch (name) {
      case "rustup":
        return Rustup(args, self);
      case "cargo":
      case "rustc":
      case "rustdoc":
        if (!Proxied(name)) return 1;
        if (name == "cargo") return Cargo(first, all, root, secret);
        if (first == "-vV") {
          Say("rustc 1.99.0 (b9 2026-10-01)\nbinary: rustc\nhost: " + Triple + "\nrelease: 1.99.0");
          return 0;
        }
        if (first == "--print") {
          Say(Triple);
          return 0;
        }
        return 1;
      case "uv":
        return Uv(first, all, pair, secret);
      case "python":
        Say("Python 3.12.12");
        return 0;
      case "go":
        return Go(args, first, all, pair, root, self, secret);
      case "gofmt":
        return 0;
      case "smoke-tool":
        Say("smoke-tool 1.2.3" + secret);
        return 0;
    }
    return 1;
  }

  // rustup verifies and unpacks the channel's components; this lays out the
  // same toolchain, with copies of itself as its binaries.
  static int Rustup(string[] args, string self) {
    if (args.Length < 3 || args[0] != "toolchain" || args[1] != "install") return 1;
    string toolchain = Path.Combine(Env("RUSTUP_HOME"), "toolchains", args[2] + "-" + Triple);
    string bin = Path.Combine(toolchain, "bin");
    string rustlib = Path.Combine(toolchain, "lib", "rustlib");
    Directory.CreateDirectory(bin);
    Directory.CreateDirectory(rustlib);
    foreach (string tool in new[] { "cargo", "rustc", "rustdoc" }) {
      File.Copy(self, Path.Combine(bin, tool + ".exe"));
    }
    File.Copy(Path.Combine(State, "rust", "manifest.toml"), Path.Combine(rustlib, "multirust-channel-manifest.toml"));
    File.WriteAllText(Path.Combine(rustlib, "components"),
      "cargo-" + Triple + "\nrust-std-" + Triple + "\nrustc-" + Triple + "\n");
    return 0;
  }

  // A rustup proxy runs the installed toolchain that RUSTUP_HOME and
  // RUSTUP_TOOLCHAIN select, and fails without one.
  static bool Proxied(string name) {
    string home = Env("RUSTUP_HOME");
    string toolchain = Env("RUSTUP_TOOLCHAIN");
    if (home != null && toolchain != null && File.Exists(Path.Combine(home, "toolchains", toolchain, "bin", name + ".exe"))) {
      return true;
    }
    Console.Error.WriteLine("stand-in rustup proxy: no installed toolchain selected for " + name);
    return false;
  }

  static int Cargo(string first, string all, string root, string secret) {
    switch (first) {
      case "metadata":
        string id = Json("path+file:///" + root.Replace('\\', '/') + "/crates/rs-app#0.1.0");
        Say("{\"packages\":[{\"id\":" + id + ",\"name\":\"rs-app\",\"source\":null,\"manifest_path\":" +
          Json(Path.Combine(root, "crates", "rs-app", "Cargo.toml")) +
          ",\"dependencies\":[],\"targets\":[{\"name\":\"rs_app\",\"kind\":[\"lib\"]}]}],\"workspace_members\":[" + id +
          "],\"target_directory\":" + Json(Path.Combine(root, "target")) + ",\"metadata\":{\"name\":\"rs-root\"}}");
        return 0;
      case "fetch":
        return all == "fetch --locked" && Env("CARGO_HOME") != null ? 0 : 1;
      case "build":
        Say("managed-cargo " + all + " toolchain=" + Env("RUSTUP_TOOLCHAIN") + secret);
        return 0;
    }
    return 1;
  }

  static int Uv(string first, string all, string pair, string secret) {
    if (all == "--version") {
      Say("uv 0.13.0");
      return 0;
    }
    // Like uv, find the requested managed interpreter in its install directory.
    if (pair == "python find") {
      string python = Path.Combine(Env("UV_PYTHON_INSTALL_DIR") ?? "", Env("UV_PYTHON") ?? "", "python.exe");
      if (!Path.IsPathRooted(python) || !File.Exists(python)) return 2;
      Say(python);
      return 0;
    }
    if (pair == "sync --locked") return 0;
    if (first == "build") {
      Say("managed-uv " + all + " python=" + Env("UV_PYTHON") + " downloads=" + Env("UV_PYTHON_DOWNLOADS") + secret);
      return 0;
    }
    return 1;
  }

  static int Go(string[] args, string first, string all, string pair, string root, string self, string secret) {
    string app = Path.Combine(root, "goapp");
    switch (pair) {
      case "work edit":
        Say("{\"Go\":\"1.22\",\"Use\":[{\"DiskPath\":\"./goapp\"}]}");
        return 0;
      case "mod edit":
        Say("{\"Module\":{\"Path\":\"example.com/goapp\"}}");
        return 0;
      case "mod graph":
      case "mod download":
        return 0;
      case "list -mod=readonly":
        Say(args.Length > 3 && args[3] == "-f"
          ? Path.Combine(app, "go.mod")
          : "{\"Path\":\"example.com/goapp\",\"Main\":true,\"Dir\":" + Json(app) + "}");
        return 0;
      case "env -json":
        string home = Env("USERPROFILE") ?? root;
        var values = new StringBuilder("{");
        for (int i = 2; i < args.Length; i++) {
          string value = args[i] == "GOOS" ? "windows"
            : args[i] == "GOCACHE" ? Path.Combine(home, "go-cache")
            : args[i] == "GOMODCACHE" ? Path.Combine(home, "go-mod") : "";
          values.Append(i > 2 ? "," : "").Append(Json(args[i])).Append(":").Append(Json(value));
        }
        Say(values.Append("}").ToString());
        return 0;
      case "version":
        Say("go version go1.22.12 windows/amd64");
        return 0;
    }
    if (first == "build") {
      string goroot = Env("GOROOT");
      string identity = Identity(self);
      bool locked = goroot != null && identity != null && identity == Identity(Path.Combine(goroot, "bin", "go.exe"));
      Say("managed-go " + all + " GOTOOLCHAIN=" + Env("GOTOOLCHAIN") + " goroot=" + (locked ? "locked" : "other") + secret);
      return 0;
    }
    return 1;
  }
}
