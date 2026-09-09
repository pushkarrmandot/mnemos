// swift-tools-version:5.9
// SPDX-License-Identifier: AGPL-3.0-or-later
import PackageDescription

let package = Package(
    name: "mnemos-audio",
    platforms: [.macOS(.v14)],
    targets: [
        .target(
            name: "MnemosAudioKit",
            path: "Sources/MnemosAudioKit"
        ),
        .executableTarget(
            name: "mnemos-audio",
            dependencies: ["MnemosAudioKit"],
            path: "Sources/mnemos-audio",
            linkerSettings: [
                // Embeds Resources/Info.plist into __TEXT,__info_plist so this
                // bare executable carries a bundle identity. Chiefly for
                // LSUIElement — without it, running from the packaged app's
                // Contents/MacOS/ makes LaunchServices treat the sidecar as an
                // app and it bounces in the Dock for the whole recording. See
                // that plist's own comment for the details.
                //
                // `unsafeFlags` only blocks use as a *dependency* of another
                // package; this is a leaf executable nothing depends on, so
                // the restriction doesn't apply.
                .unsafeFlags([
                    "-Xlinker", "-sectcreate",
                    "-Xlinker", "__TEXT",
                    "-Xlinker", "__info_plist",
                    "-Xlinker", "Resources/Info.plist",
                ])
            ]
        ),
        .executableTarget(
            name: "mnemos-meeting-watcher",
            dependencies: ["MnemosAudioKit"],
            path: "Sources/mnemos-meeting-watcher",
            linkerSettings: [
                // Same embedded identity as mnemos-audio, same reasons
                // (no Dock bounce, TCC attributes to the host app) — and
                // literally the same Resources/Info.plist file, since both
                // executables need identical CFBundleIdentifier/LSUIElement
                // and neither needs a usage-description string this one
                // doesn't already have unused (this process never reads
                // audio content, only per-process activity *state*, which
                // needs no consent — verified empirically, see the source
                // header comment).
                .unsafeFlags([
                    "-Xlinker", "-sectcreate",
                    "-Xlinker", "__TEXT",
                    "-Xlinker", "__info_plist",
                    "-Xlinker", "Resources/Info.plist",
                ])
            ]
        )
    ]
)
