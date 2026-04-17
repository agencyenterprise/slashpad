class Slashpad < Formula
  desc "Desktop AI command palette powered by Claude"
  homepage "https://github.com/agencyenterprise/slashpad"
  url "https://github.com/agencyenterprise/slashpad/archive/refs/tags/v0.1.25.tar.gz"
  sha256 "a33bfcd4b6a5ac29d199070570ccb9f82334670a235fd82ad7866441f5b0486b"
  license "MIT"

  # To update: change BUN_VERSION, then run:
  #   curl -sL "https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/bun-darwin-aarch64.zip" | shasum -a 256
  #   curl -sL "https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/bun-darwin-x64.zip" | shasum -a 256
  BUN_VERSION = "1.3.12"

  depends_on :macos

  resource "bun" do
    if Hardware::CPU.arm?
      url "https://github.com/oven-sh/bun/releases/download/bun-v#{BUN_VERSION}/bun-darwin-aarch64.zip"
      sha256 "6c4bb87dd013ed1a8d6a16e357a3d094959fd5530b4d7061f7f3680c3c7cea1c"
    else
      url "https://github.com/oven-sh/bun/releases/download/bun-v#{BUN_VERSION}/bun-darwin-x64.zip"
      sha256 "0f58c53a3e7947f1e626d2f8d285f97c14b7cadcca9c09ebafc0ae9d35b58c3d"
    end
  end

  resource "slashpad-binary" do
    if Hardware::CPU.arm?
      url "https://github.com/agencyenterprise/slashpad/releases/download/v0.1.25/slashpad-darwin-aarch64"
      sha256 "34f2e53ddfd30df182a0968805ac68b81b3e1e317b8f29d4fa7d190a6521f0cb"
    else
      url "https://github.com/agencyenterprise/slashpad/releases/download/v0.1.25/slashpad-darwin-x86_64"
      sha256 "ff0412575530fc2a24b2183b3d439a18be8dfbf9a665550f514e94b739080dee"
    end
  end

  def install
    resource("slashpad-binary").stage do
      if Hardware::CPU.arm?
        bin.install "slashpad-darwin-aarch64" => "slashpad"
      else
        bin.install "slashpad-darwin-x86_64" => "slashpad"
      end
      chmod 0755, bin/"slashpad"
    end

    libexec.install "agent"
    libexec.install "package.json"

    resource("bun").stage do
      (libexec/"bin").install "bun"
      chmod 0755, libexec/"bin/bun"
    end

    cd libexec do
      system libexec/"bin/bun", "install", "--production"
    end
  end

  service do
    run opt_bin/"slashpad"
    keep_alive crashed: true
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
    assert_predicate libexec/"bin/bun", :executable?
  end
end
