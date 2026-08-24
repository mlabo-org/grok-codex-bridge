import Foundation

let fileManager = FileManager.default
let launcherArguments = Array(CommandLine.arguments.dropFirst())
let bridge = launcherArguments.first.map { URL(fileURLWithPath: $0) }
let arguments = Array(launcherArguments.dropFirst())
let logDirectory = fileManager.homeDirectoryForCurrentUser
    .appendingPathComponent("Library/Application Support/grok-codex-bridge/logs")
let logURL = logDirectory.appendingPathComponent("mode-switch.log")

func timestamp() -> String {
    ISO8601DateFormatter().string(from: Date())
}

func appendLog(_ message: String) {
    let bytes = Data("\(timestamp()) launcher \(message)\n".utf8)
    if !fileManager.fileExists(atPath: logURL.path) {
        fileManager.createFile(atPath: logURL.path, contents: nil)
    }
    guard let handle = try? FileHandle(forWritingTo: logURL) else {
        return
    }
    defer { try? handle.close() }
    do {
        try handle.seekToEnd()
        try handle.write(contentsOf: bytes)
    } catch {
        return
    }
}

guard let bridge, bridge.path.hasPrefix("/"), arguments.first == "switch" else {
    appendLog("refused invalid command")
    exit(EXIT_FAILURE)
}

let bridgeValues = try? bridge.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
guard bridgeValues?.isRegularFile == true,
      bridgeValues?.isSymbolicLink != true,
      fileManager.isExecutableFile(atPath: bridge.path) else {
    appendLog("missing sibling bridge executable")
    exit(EXIT_FAILURE)
}

do {
    try fileManager.createDirectory(at: logDirectory, withIntermediateDirectories: true)
    if !fileManager.fileExists(atPath: logURL.path) {
        fileManager.createFile(atPath: logURL.path, contents: nil)
    }
    appendLog("started pid=\(ProcessInfo.processInfo.processIdentifier)")
    let logHandle = try FileHandle(forWritingTo: logURL)
    try logHandle.seekToEnd()

    let coordinator = Process()
    coordinator.executableURL = bridge
    coordinator.arguments = arguments
    coordinator.standardInput = FileHandle.nullDevice
    coordinator.standardOutput = logHandle
    coordinator.standardError = logHandle
    try coordinator.run()
    coordinator.waitUntilExit()
    try logHandle.close()

    appendLog("finished exit=\(coordinator.terminationStatus)")
    exit(coordinator.terminationStatus)
} catch {
    appendLog("failed error=\(error.localizedDescription)")
    exit(EXIT_FAILURE)
}
