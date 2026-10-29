// AiClub 窗口取证小工具(macOS)。
//
// 用法:
//   swift tools/winprobe.swift list                列出所有窗口的 id / owner / 标题 / 位置
//   swift tools/winprobe.swift find <ownerSubstr>  打印匹配窗口的 id 与 bounds(JSON)
//   swift tools/winprobe.swift drag <x> <y> <dx> <dy>   从 (x,y) 拖动 (dx,dy)
//   swift tools/winprobe.swift click <x> <y>       在 (x,y) 单击
//   swift tools/winprobe.swift dblclick <x> <y>    在 (x,y) 双击
//
// 坐标系:CoreGraphics 全局坐标(左上角为原点,与 screencapture 的 -R 一致)。

import CoreGraphics
import Foundation

struct WinInfo {
    let id: Int
    let owner: String
    let title: String
    let x: Double
    let y: Double
    let w: Double
    let h: Double
    let layer: Int
}

func allWindows(onScreenOnly: Bool = true) -> [WinInfo] {
    let opts: CGWindowListOption = onScreenOnly
        ? CGWindowListOption(arrayLiteral: .optionOnScreenOnly, .excludeDesktopElements)
        : CGWindowListOption(arrayLiteral: .optionAll, .excludeDesktopElements)
    guard let raw = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] else {
        return []
    }
    return raw.compactMap { d in
        guard let num = d[kCGWindowNumber as String] as? Int,
              let owner = d[kCGWindowOwnerName as String] as? String
        else { return nil }
        let title = (d[kCGWindowName as String] as? String) ?? ""
        let layer = (d[kCGWindowLayer as String] as? Int) ?? 0
        var x = 0.0, y = 0.0, w = 0.0, h = 0.0
        if let b = d[kCGWindowBounds as String] as? [String: Any] {
            x = (b["X"] as? NSNumber)?.doubleValue ?? 0
            y = (b["Y"] as? NSNumber)?.doubleValue ?? 0
            w = (b["Width"] as? NSNumber)?.doubleValue ?? 0
            h = (b["Height"] as? NSNumber)?.doubleValue ?? 0
        }
        return WinInfo(id: num, owner: owner, title: title, x: x, y: y, w: w, h: h, layer: layer)
    }
}

func postMouse(_ type: CGEventType, _ p: CGPoint) {
    guard let e = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: p, mouseButton: .left) else { return }
    e.post(tap: .cghidEventTap)
}

func drag(from: CGPoint, to: CGPoint) {
    // 移到起点,按下,分步移动(系统需要看到中间移动才会认为是拖动),
    // 停一下再松开。
    postMouse(.mouseMoved, from)
    usleep(120_000)
    postMouse(.leftMouseDown, from)
    usleep(120_000)
    let steps = 24
    for i in 1...steps {
        let t = Double(i) / Double(steps)
        let p = CGPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t)
        postMouse(.leftMouseDragged, p)
        usleep(12_000)
    }
    usleep(150_000)
    postMouse(.leftMouseUp, to)
    usleep(120_000)
}

func click(_ p: CGPoint, count: Int) {
    postMouse(.mouseMoved, p)
    usleep(120_000)
    for i in 1...count {
        let down = CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: p, mouseButton: .left)!
        down.setIntegerValueField(.mouseEventClickState, value: Int64(i))
        down.post(tap: .cghidEventTap)
        usleep(40_000)
        let up = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: p, mouseButton: .left)!
        up.setIntegerValueField(.mouseEventClickState, value: Int64(i))
        up.post(tap: .cghidEventTap)
        usleep(60_000)
    }
}

func cursorPos() -> CGPoint {
    CGEvent(source: nil)?.location ?? CGPoint(x: -1, y: -1)
}

let args = Array(CommandLine.arguments.dropFirst())
guard let cmd = args.first else {
    FileHandle.standardError.write("usage: winprobe.swift <list|find|drag|click|dblclick> ...\n".data(using: .utf8)!)
    exit(2)
}

switch cmd {
case "list":
    for w in allWindows().sorted(by: { $0.layer < $1.layer }) {
        print("id=\(w.id) layer=\(w.layer) owner=\(w.owner) title=\"\(w.title)\" bounds=(\(Int(w.x)),\(Int(w.y)),\(Int(w.w)),\(Int(w.h)))")
    }
case "listall":
    // 含离屏/其他 Space 的窗口:用来判断"窗口是没了还是被挪走了"
    for w in allWindows(onScreenOnly: false).sorted(by: { $0.layer < $1.layer }) {
        print("id=\(w.id) layer=\(w.layer) owner=\(w.owner) title=\"\(w.title)\" bounds=(\(Int(w.x)),\(Int(w.y)),\(Int(w.w)),\(Int(w.h)))")
    }
case "find":
    guard args.count >= 2 else { exit(2) }
    let needle = args[1].lowercased()
    let hits = allWindows().filter {
        $0.owner.lowercased().contains(needle) || $0.title.lowercased().contains(needle)
    }
    if hits.isEmpty { print("{\"found\":false}"); exit(1) }
    for w in hits {
        print("{\"found\":true,\"id\":\(w.id),\"owner\":\"\(w.owner)\",\"title\":\"\(w.title)\",\"x\":\(w.x),\"y\":\(w.y),\"w\":\(w.w),\"h\":\(w.h)}")
    }
case "drag":
    guard args.count >= 5,
          let x = Double(args[1]), let y = Double(args[2]),
          let dx = Double(args[3]), let dy = Double(args[4]) else { exit(2) }
    drag(from: CGPoint(x: x, y: y), to: CGPoint(x: x + dx, y: y + dy))
case "click", "dblclick":
    guard args.count >= 3, let x = Double(args[1]), let y = Double(args[2]) else { exit(2) }
    click(CGPoint(x: x, y: y), count: cmd == "click" ? 1 : 2)
case "cursor":
    let p = cursorPos()
    print("cursor=(\(Int(p.x)),\(Int(p.y)))")
case "selftest":
    // 合成事件是否真的生效?把光标挪到 (7,7) 再读回来。
    // 读回来是 (7,7) ⇒ CGEventPost 有 Accessibility 权限,后续 drag 结果可信;
    // 读回来还是原位 ⇒ 事件被系统丢弃,任何"拖不动"的结论都不成立。
    let before = cursorPos()
    postMouse(.mouseMoved, CGPoint(x: 7, y: 7))
    usleep(250_000)
    let after = cursorPos()
    let ok = abs(after.x - 7) < 2 && abs(after.y - 7) < 2
    postMouse(.mouseMoved, before)
    print("{\"synthetic_input\":\(ok),\"before\":[\(Int(before.x)),\(Int(before.y))],\"after\":[\(Int(after.x)),\(Int(after.y))]}")
    exit(ok ? 0 : 1)
default:
    FileHandle.standardError.write("unknown command \(cmd)\n".data(using: .utf8)!)
    exit(2)
}
