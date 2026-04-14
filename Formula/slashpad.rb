class Slashpad < Formula
  desc "Desktop AI command palette powered by Claude"
  homepage "https://github.com/agencyenterprise/slashpad"
  url "https://github.com/agencyenterprise/slashpad/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "eb3ca1cb65f71b6575edd5af57b5eb8ed9385188fa5b0adb218cd7ae83b0d82d"
  license "MIT"

  depends_on "rust" => :build
  depends_on "node"
  depends_on :macos

  def install
    system "cargo", "install", *std_cargo_args
    libexec.install "agent"
    libexec.install "package.json"
    if File.exist?("package-lock.json")
      libexec.install "package-lock.json"
    end
    cd libexec do
      system "npm", "install", "--production"
    end
  end

  service do
    run opt_bin/"slashpad"
    keep_alive true
    log_path var/"log/slashpad.log"
    error_log_path var/"log/slashpad.log"
  end

  def caveats
    <<~EOS
      To start Slashpad now and have it launch at login:
        brew services start slashpad

      Authenticate with Claude by running `claude login` in your terminal,
      then press Ctrl+Space to open the Slashpad palette.

      To use an Anthropic API key instead, click the Slashpad icon in the
      menu bar to open Settings and uncheck "Use Claude subscription".
    EOS
  end

  test do
    assert_predicate bin/"slashpad", :executable?
  end
end
