// swift-tools-version: 5.9
import PackageDescription
import Foundation
let rust=URL(fileURLWithPath:#filePath).deletingLastPathComponent().appendingPathComponent(".build/rust/release").path
let package=Package(name:"Ruycast",platforms:[.macOS(.v14)],products:[.executable(name:"RuycastApp",targets:["RuycastApp"])],targets:[
    .target(name:"LauncherCore",linkerSettings:[.unsafeFlags(["-L",rust]),.linkedLibrary("rustcast_core"),.linkedLibrary("iconv"),.linkedLibrary("resolv")]),
    .executableTarget(name:"RuycastApp",dependencies:["LauncherCore"]),
    .executableTarget(name:"LauncherCoreChecks",dependencies:["LauncherCore"],path:"Tests/LauncherCoreTests")
])
