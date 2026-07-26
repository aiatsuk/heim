# Homebrew formula template (tap: e.g. aiatsuk/homebrew-tap).
#
# After a GitHub Release is published for tag vX.Y.Z:
# 1. Download the macOS tarballs from the release
# 2. shasum -a 256 the arm64 / x64 archives
# 3. Replace VERSION / sha256 placeholders below
# 4. Publish in your tap as Formula/heim.rb
#
# Usage once tapped:
#   brew install aiatsuk/tap/heim

class Heim < Formula
  desc "Stop vibe-code bloat: real-time LOC/size/git deltas + JSON agents can self-audit"
  homepage "https://github.com/aiatsuk/heim"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/aiatsuk/heim/releases/download/v#{version}/heim-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_ARM64_SHA256"
    end
    on_intel do
      url "https://github.com/aiatsuk/heim/releases/download/v#{version}/heim-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X64_SHA256"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/aiatsuk/heim/releases/download/v#{version}/heim-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_LINUX_AMD64_SHA256"
    end
  end

  def install
    bin.install "heim"
  end

  test do
    assert_match "heim", shell_output("#{bin}/heim --help")
    system "#{bin}/heim", "--once", "--json", testpath
  end
end
