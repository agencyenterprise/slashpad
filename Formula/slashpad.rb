class Slashpad < Formula
  desc "Desktop AI command palette powered by Claude"
  homepage "https://github.com/agencyenterprise/slashpad"
  url "https://github.com/agencyenterprise/slashpad/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "PLACEHOLDER"
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

  def caveats
    <<~EOS
      Slashpad requires an Anthropic API key. Launch the app and press
      Ctrl+Space to open the palette, then type /settings to configure it.

      Grant Accessibility permissions in System Settings > Privacy & Security
      for the global hotkey to work.
    EOS
  end

  test do
    assert_predicate bin/"slashpad", :executable?
  end
end
