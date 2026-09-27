"""Representative generated operations, checked against independent API reads."""

import base64
import json


def run_generated(api, yarr, info_hash, payload, torrent, wait_for):
    checked = []

    def call(name, args=None):
        result = yarr.action(name, args)
        checked.append(name)
        return result

    def row():
        return api.get('/api/v2/torrents/info?hashes=' + info_hash)[0]

    def files():
        return api.get('/api/v2/torrents/files?hash=' + info_hash)

    properties = call('get_torrent_properties', {'hash': info_hash})
    assert properties['total_size'] == payload.stat().st_size
    assert len(call('get_torrent_files', {'hash': info_hash})) == 1
    assert isinstance(call('get_torrent_trackers', {'hash': info_hash}), list)
    assert isinstance(call('get_sync_torrent_peers', {'hash': info_hash}), dict)
    assert isinstance(call('get_search_plugins'), list)
    assert isinstance(call('get_rss_items'), dict)
    assert isinstance(call('get_log_main', {'last_known_id': -1}), list)

    call('post_app_set_preferences', {'body': {'json': json.dumps({
        'queueing_enabled': True, 'max_active_downloads': 5, 'save_path': '/downloads',
    })}})
    preferences = api.get('/api/v2/app/preferences')
    assert preferences['queueing_enabled'] and preferences['max_active_downloads'] == 5
    assert call('get_app_preferences')['max_active_downloads'] == 5

    call('post_torrents_file_prio', {'body': {'hash': info_hash, 'id': '0', 'priority': 6}})
    wait_for('file priority', lambda: files()[0]['priority'] == 6)
    call('post_torrents_top_prio', {'body': {'hashes': info_hash}})

    call('post_torrents_set_share_limits', {'body': {
        'hashes': info_hash, 'ratioLimit': 2.5, 'seedingTimeLimit': -1,
        'inactiveSeedingTimeLimit': -1, 'shareLimitAction': 'Stop',
    }})
    wait_for('share ratio limit', lambda: row()['ratio_limit'] == 2.5)
    for enabled in (True, False):
        call('post_torrents_set_force_start', {'body': {'hashes': info_hash, 'value': enabled}})
        wait_for('force-start state', lambda: row()['force_start'] == enabled)
    call('post_torrents_stop', {'body': {'hashes': info_hash}})
    for enabled in (True, False):
        call('post_torrents_set_auto_management', {'body': {'hashes': info_hash, 'enable': enabled}})
        wait_for('automatic management', lambda: row()['auto_tmm'] == enabled)

    for location in ('/downloads/moved', '/downloads'):
        call('post_torrents_set_location', {'body': {'hashes': info_hash, 'location': location}})
        wait_for('torrent storage location', lambda: row()['save_path'].rstrip('/') == location)
    renamed = payload.stem + '-renamed.bin'
    for old, new in ((payload.name, renamed), (renamed, payload.name)):
        call('post_torrents_rename_file', {'body': {'hash': info_hash, 'oldPath': old, 'newPath': new}})
        wait_for('torrent filename', lambda: files()[0]['name'] == new)
    call('post_torrents_rename', {'body': {'hash': info_hash, 'name': 'Synthetic lab torrent'}})
    wait_for('torrent display name', lambda: row()['name'] == 'Synthetic lab torrent')

    tracker = 'http://127.0.0.1:1/announce'
    changed_tracker = 'http://127.0.0.1:2/announce'

    def trackers():
        return {entry['url'] for entry in api.get('/api/v2/torrents/trackers?hash=' + info_hash)}

    call('post_torrents_add_trackers', {'body': {'hash': info_hash, 'urls': tracker}})
    wait_for('tracker addition', lambda: tracker in trackers())
    call('post_torrents_edit_tracker', {'body': {'hash': info_hash, 'url': tracker, 'newUrl': changed_tracker}})
    wait_for('tracker edit', lambda: changed_tracker in trackers() and tracker not in trackers())
    call('post_torrents_reannounce', {'body': {'hashes': info_hash}})
    call('post_torrents_remove_trackers', {'body': {'hash': info_hash, 'urls': changed_tracker}})
    wait_for('tracker removal', lambda: changed_tracker not in trackers())
    call('post_torrents_recheck', {'body': {'hashes': info_hash}})
    wait_for('torrent recheck', lambda: row()['progress'] == 1 and not row()['state'].startswith('checking'))

    exported = call('get_torrent_export', {'hash': info_hash})
    assert len(base64.b64decode(exported['base64'])) > 50
    parsed = call('post_torrents_parse_metadata', {
        'multipartFileBase64': base64.b64encode(torrent.read_bytes()).decode(),
        'fileName': torrent.name,
    })
    assert isinstance(parsed, list) and len(parsed) == 1
    return sorted(set(checked))
