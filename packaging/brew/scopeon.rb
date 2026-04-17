class Scopeon < Formula
  desc "AI context observability for Claude Code, Codex, Cursor, and every LLM coding tool"
  homepage "https://github.com/sorunokoe/Scopeon"
  version "0.6.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/sorunokoe/Scopeon/releases/download/v#{version}/scopeon-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_AARCH64_APPLE_DARWIN"
    end
    on_intel do
      url "https://github.com/sorunokoe/Scopeon/releases/download/v#{version}/scopeon-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_X86_64_APPLE_DARWIN"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/sorunokoe/Scopeon/releases/download/v#{version}/scopeon-aarch64-unknown-linux-musl.tar.gz"
      sha256 "PLACEHOLDER_AARCH64_LINUX"
    end
    on_intel do
      url "https://github.com/sorunokoe/Scopeon/releases/download/v#{version}/scopeon-x86_64-unknown-linux-musl.tar.gz"
      sha256 "PLACEHOLDER_X86_64_LINUX"
    end
  end

  def install
    bin.install "scopeon"
  end

  test do
    assert_match "scopeon", shell_output("#{bin}/scopeon --version")
  end
end
