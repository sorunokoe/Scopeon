class Scopeon < Formula
  desc "AI context observability for Claude Code, Codex, Cursor, and every LLM coding tool"
  homepage "https://github.com/sorunokoe/Scopeon"
  version "0.6.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/sorunokoe/Scopeon/releases/download/v#{version}/scopeon-aarch64-apple-darwin.tar.gz"
      sha256 "ba08d27cd6148c774f1c81835c6cf98ea7ca3864dee47be16e7f21b07b9346aa"
    end
    on_intel do
      url "https://github.com/sorunokoe/Scopeon/releases/download/v#{version}/scopeon-x86_64-apple-darwin.tar.gz"
      sha256 "74988a20e939227236de349aaf2f32c7ececf0ff46b4cf864c2f5e83e32e9ddc"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/sorunokoe/Scopeon/releases/download/v#{version}/scopeon-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "9eec5a7f0a3283bd2c927d2e80fda4d7d485ee239a14db398b0414a2a63852d4"
    end
    on_intel do
      url "https://github.com/sorunokoe/Scopeon/releases/download/v#{version}/scopeon-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "773170bd32e97092b9429194f7f0217f363e187880b71be35e60c044ac023c17"
    end
  end

  def install
    bin.install "scopeon"
  end

  test do
    assert_match "scopeon", shell_output("#{bin}/scopeon --version")
  end
end
