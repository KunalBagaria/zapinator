# Homebrew cask template. scripts/release.sh fills in the version and
# checksum and pushes the result to KunalBagaria/homebrew-zapinator.
cask "zapinator" do
  version "__VERSION__"
  sha256 "__SHA256__"

  url "https://zapinator.kunalbagaria.com/releases/NotchZapInator-#{version}.tar.gz"
  name "NOTCH-ZAP-INATOR"
  desc "Mad-scientist notch animation for when your MacBook starts charging"
  homepage "https://zapinator.kunalbagaria.com/"

  depends_on arch: :arm64
  depends_on macos: :ventura

  # Not `app` + `postflight`: Homebrew sandboxes post-install steps away from
  # ~/Library, where the LaunchAgent goes. agent.sh copies the app into
  # appdir, clears the download quarantine (it's ad-hoc signed, not
  # notarized), starts the LaunchAgent and mutes macOS's charging chime. It
  # goes through /bin/bash because macOS kills scripts executed straight out
  # of a quarantined, un-notarized bundle.
  installer script: {
    executable: "/bin/bash",
    args:       ["#{staged_path}/NotchZapInator.app/Contents/Resources/agent.sh", "install", "--into", appdir.to_s],
  }

  uninstall launchctl: "com.kunalbagaria.zapinator",
            script:    {
              executable:   "/bin/bash",
              args:         ["#{appdir}/NotchZapInator.app/Contents/Resources/agent.sh", "uninstall"],
              must_succeed: false,
            },
            delete:    "#{appdir}/NotchZapInator.app"

  zap trash: [
    "~/Library/Application Support/NotchZapInator",
    "~/Library/Logs/NotchZapInator.log",
  ]

  caveats <<~EOS
    NOTCH-ZAP-INATOR runs in the background and starts at login. Plug in your
    charger to see it, or preview it with:
      #{appdir}/NotchZapInator.app/Contents/MacOS/notch-zap-inator --demo

    It mutes macOS's own charging chime; `brew uninstall` puts it back. To
    keep the chime now:
      bash #{appdir}/NotchZapInator.app/Contents/Resources/chime.sh restore
  EOS
end
