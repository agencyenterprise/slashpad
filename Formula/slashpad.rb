class Slashpad < Formula
  desc "Desktop AI command palette powered by Claude"
  homepage "https://github.com/agencyenterprise/slashpad"
  url "https://github.com/agencyenterprise/slashpad/archive/refs/tags/v0.1.16.tar.gz"
  sha256 "e9af60cd819b0c0c00e2b05547ba18a721c77af6c733600634df025beb714e6a"
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
      url "https://github.com/agencyenterprise/slashpad/releases/download/v0.1.16/slashpad-darwin-aarch64"
      sha256 "dfdcd292dbcd4f0b1aeded09e60e9fb09170da1a8184404a660781aa9e04f3b1"
    else
      url "https://github.com/agencyenterprise/slashpad/releases/download/v0.1.16/slashpad-darwin-x86_64"
      sha256 "3bc33aeabdbe668dd56de4a92120183f4e27e42f0d77da5b94e142be7c1fa0f4"
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
