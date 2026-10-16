import AppKit
import Foundation

// The SDL engine is a child process so Finder can launch it without terminal
// arguments. Logs and user state stay outside the read-only application bundle.
let application = NSApplication.shared
application.setActivationPolicy(.accessory)
let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    .appendingPathComponent("Ikura", isDirectory: true)
let defaults = UserDefaults.standard
let arguments = Array(CommandLine.arguments.dropFirst())
let bundle = Bundle.main.bundleURL
let contents = bundle.appendingPathComponent("Contents", isDirectory: true)
let resources = contents.appendingPathComponent("Resources", isDirectory: true)
let engine = contents.appendingPathComponent("MacOS/ikura-engine")

func alert(_ message: String, _ detail: String) {
    application.activate(ignoringOtherApps: true)
    let panel = NSAlert()
    panel.messageText = message
    panel.informativeText = detail
    panel.alertStyle = .warning
    panel.addButton(withTitle: "好")
    panel.runModal()
}
func chooseData() -> URL? {
    application.activate(ignoringOtherApps: true)
    let panel = NSOpenPanel()
    panel.title = "选择 Ikura 游戏数据目录"
    panel.message = "请选择已准备好的 game_data：包含 scripts.json、PNG-images、OGG-audio 和 DATA。"
    panel.canChooseDirectories = true
    panel.canChooseFiles = false
    panel.allowsMultipleSelection = false
    panel.prompt = "使用此目录"
    return panel.runModal() == .OK ? panel.url : nil
}
func validData(_ directory: URL) -> Bool {
    ["scripts.json", "PNG-images", "OGG-audio", "DATA"].allSatisfy {
        FileManager.default.fileExists(atPath: directory.appendingPathComponent($0).path)
    }
}

do {
    try FileManager.default.createDirectory(at: support, withIntermediateDirectories: true)
    let interactive = arguments.isEmpty
    let changeData = interactive && NSEvent.modifierFlags.contains(.option)
    var data: URL?
    if let first = arguments.first {
        data = URL(fileURLWithPath: first, isDirectory: true).standardizedFileURL
    } else if !changeData, let path = defaults.string(forKey: "GameDataDirectory") {
        let previous = URL(fileURLWithPath: path, isDirectory: true)
        if validData(previous) { data = previous }
    }
    while data == nil {
        guard let selected = chooseData() else { exit(0) }
        if validData(selected) { data = selected }
        else { alert("游戏数据不完整", "该目录需要包含 scripts.json、PNG-images、OGG-audio、DATA。准备方法见安装包中的 README。") }
    }
    let directory = data!
    if interactive { defaults.set(directory.path, forKey: "GameDataDirectory") }
    var language = ProcessInfo.processInfo.environment["IKURA_LANG"]
        ?? defaults.string(forKey: "Language") ?? "ja"
    if interactive && (changeData || defaults.string(forKey: "Language") == nil) {
        if FileManager.default.fileExists(atPath: directory.appendingPathComponent("zh-CN.json").path) {
            let panel = NSAlert()
            panel.messageText = "选择游戏语言"
            panel.informativeText = "已发现本机中文文本包。按住 Option 打开应用可以重新选择目录和语言。"
            panel.addButton(withTitle: "简体中文")
            panel.addButton(withTitle: "日本語")
            language = panel.runModal() == .alertFirstButtonReturn ? "zh-CN" : "ja"
        } else { language = "ja" }
        defaults.set(language, forKey: "Language")
    }
    var environment = ProcessInfo.processInfo.environment
    environment["IKURA_LANG"] = language
    environment["IKURA_FFMPEG"] = resources.appendingPathComponent("tools/ffmpeg").path
    environment["IKURA_FFPROBE"] = resources.appendingPathComponent("tools/ffprobe").path
    if environment["IKURA_TRANSLATIONS"] == nil {
        environment["IKURA_TRANSLATIONS"] = directory.appendingPathComponent("zh-CN.json").path
    }
    if environment["IKURA_DATA"] == nil && FileManager.default.fileExists(atPath: directory.appendingPathComponent("DATA").path) {
        environment["IKURA_DATA"] = directory.appendingPathComponent("DATA").path
    }
    if environment["IKURA_SAVE_DIR"] == nil {
        // Separate games and languages; moving a prepared data folder keeps its
        // saves by retaining the same scripts.json fingerprint.
        let scripts = directory.appendingPathComponent("scripts.json")
        let bytes = try Data(contentsOf: scripts)
        let hash = bytes.reduce(UInt64(0xcbf29ce484222325)) { ($0 ^ UInt64($1)) &* 0x100000001b3 }
        let name = "saves/\(String(hash, radix: 16))/\(language)"
        environment["IKURA_SAVE_DIR"] = support.appendingPathComponent(name).path
    }
    let process = Process()
    process.executableURL = engine
    process.currentDirectoryURL = directory
    process.environment = environment
    if interactive {
        process.arguments = [directory.path, directory.appendingPathComponent("scripts.json").path]
    } else {
        process.arguments = arguments.count == 1
            ? [directory.path, directory.appendingPathComponent("scripts.json").path]
            : arguments
    }
    let log = support.appendingPathComponent("last-launch.log")
    if interactive {
        FileManager.default.createFile(atPath: log.path, contents: nil)
        let handle = try FileHandle(forWritingTo: log)
        process.standardOutput = handle
        process.standardError = handle
    }
    try process.run()
    process.waitUntilExit()
    if process.terminationStatus != 0 && interactive {
        let tail = (try? String(contentsOf: log, encoding: .utf8))?.suffix(1800) ?? ""
        alert("游戏未能启动", "详细日志：\(log.path)\n\n\(tail)")
    }
    exit(process.terminationStatus)
} catch {
    if arguments.isEmpty { alert("Ikura 启动失败", error.localizedDescription) }
    else { fputs("Ikura launcher: \(error)\n", stderr) }
    exit(1)
}
