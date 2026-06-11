"""
MkDocs hook to inject version information into the documentation.

This hook extracts version information from:
1. Environment variable KUBARR_VERSION (if set)
2. Git tags (latest tag matching v*.*.*)
3. Fallback to "dev"

The version info is added to:
- Site metadata (extra.version_info)
- Copyright footer
- Page headers
"""

import os
import subprocess
import re
from datetime import datetime, timezone


def get_version_info():
    """Extract version information from environment or git."""
    version = os.environ.get('KUBARR_VERSION', '')
    channel = os.environ.get('KUBARR_CHANNEL', 'dev')

    if not version:
        # Try to get version from git tags
        try:
            # Get the latest tag
            result = subprocess.run(
                ['git', 'describe', '--tags', '--abbrev=0'],
                capture_output=True,
                text=True,
                check=False
            )
            if result.returncode == 0:
                tag = result.stdout.strip()
                # Extract version from tag (e.g., v1.2.3 -> 1.2.3)
                version_match = re.match(r'v?(\d+\.\d+\.\d+)', tag)
                if version_match:
                    version = version_match.group(1)

                    # Determine channel from tag
                    if re.match(r'v?\d+\.\d+\.\d+$', tag):
                        channel = 'stable'
                    elif re.match(r'v?\d+\.\d+\.\d+-(rc|beta)', tag):
                        channel = 'release'
                    else:
                        channel = 'dev'
        except Exception:
            pass

    # Get commit hash
    commit_hash = os.environ.get('KUBARR_COMMIT', '')
    if not commit_hash:
        try:
            result = subprocess.run(
                ['git', 'rev-parse', '--short', 'HEAD'],
                capture_output=True,
                text=True,
                check=False
            )
            if result.returncode == 0:
                commit_hash = result.stdout.strip()
        except Exception:
            commit_hash = 'unknown'

    # Fallback values
    if not version:
        version = '0.0.0'

    return {
        'version': version,
        'channel': channel,
        'commit': commit_hash,
        'build_date': datetime.now(timezone.utc).strftime('%Y-%m-%d')
    }


def on_config(config, **kwargs):
    """Called when MkDocs config is loaded."""
    # Get version info
    version_info = get_version_info()

    # Add to extra config for templates
    if 'extra' not in config:
        config['extra'] = {}
    config['extra']['version_info'] = version_info

    # Update copyright with version info
    channel_emoji = {
        'stable': '🟢',
        'release': '🟡',
        'dev': '🔵'
    }.get(version_info['channel'], '🔵')

    version_str = f"{channel_emoji} v{version_info['version']} ({version_info['channel']}) • {version_info['commit']}"

    if 'copyright' in config and config['copyright']:
        config['copyright'] += f" • {version_str}"
    else:
        config['copyright'] = f"© 2026 Kubarr Contributors • {version_str}"

    return config


def on_page_markdown(markdown, page, config, files):
    """Called when page markdown is loaded, before rendering."""
    # Replace version placeholders in markdown
    version_info = config.get('extra', {}).get('version_info', {})

    markdown = markdown.replace('{{VERSION}}', version_info.get('version', '0.0.0'))
    markdown = markdown.replace('{{CHANNEL}}', version_info.get('channel', 'dev'))
    markdown = markdown.replace('{{COMMIT}}', version_info.get('commit', 'unknown'))

    return markdown
