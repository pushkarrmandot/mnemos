// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "mnemos-audio",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "mnemos-audio",
            path: "Sources/mnemos-audio"
        )
    ]
)
