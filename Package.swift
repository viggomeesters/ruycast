// swift-tools-version: 6.2
import PackageDescription
import Foundation
let rust=URL(fileURLWithPath:#filePath).deletingLastPathComponent().appendingPathComponent(".build/rust/release").path
let package=Package(name:"Ruycast",platforms:[.macOS(.v14)],products:[.executable(name:"RuycastApp",targets:["RuycastApp"])],targets:[
    .target(name:"LauncherModels"),
    .target(name:"CRuycastCore",publicHeadersPath:"include"),
    .target(name:"LauncherCore",dependencies:["LauncherModels","CRuycastCore"],linkerSettings:[.unsafeFlags(["-L",rust]),.linkedLibrary("rustcast_core"),.linkedLibrary("iconv"),.linkedLibrary("resolv")]),
    .executableTarget(name:"RuycastApp",dependencies:["LauncherCore"],swiftSettings:[.unsafeFlags(["-Osize"],.when(configuration:.release))]),
    .executableTarget(name:"LauncherCoreChecks",dependencies:["LauncherCore"],path:"Tests/LauncherCoreTests")
])
