"""Release operations are mocked; these tests never write to GitHub."""
import unittest
from unittest.mock import call, patch

import release


class ReleaseTagTests(unittest.TestCase):
    @patch.object(release, 'gh')
    def test_new_tag_uses_the_validated_commit(self, gh):
        gh.side_effect = [None, {'ref': 'refs/tags/v0.0.2'}]
        release.ensure_tag('v0.0.2', 'validated-source')
        self.assertEqual(gh.call_args_list, [
            call('api', f'repos/{release.REPO}/git/ref/tags/v0.0.2', optional=True),
            call('api', f'repos/{release.REPO}/git/refs', '--method', 'POST', '--input', '-',
                 data={'ref': 'refs/tags/v0.0.2', 'sha': 'validated-source'}),
        ])

    @patch.object(release, 'gh')
    def test_matching_lightweight_or_annotated_tag_is_reused(self, gh):
        for object_type in ('commit', 'tag'):
            gh.reset_mock(side_effect=True)
            gh.side_effect = [{'object': {'type': object_type}}, {'sha': 'validated-source'}]
            release.ensure_tag('v0.0.2', 'validated-source')
            self.assertEqual(gh.call_args_list, [
                call('api', f'repos/{release.REPO}/git/ref/tags/v0.0.2', optional=True),
                call('api', f'repos/{release.REPO}/commits/v0.0.2'),
            ])

    @patch.object(release, 'gh')
    def test_different_existing_source_is_refused_without_writes(self, gh):
        gh.side_effect = [{'ref': 'refs/tags/v0.0.2'}, {'sha': 'other-source'}]
        with self.assertRaisesRegex(AssertionError, 'different source'):
            release.ensure_tag('v0.0.2', 'validated-source')
        self.assertEqual(gh.call_count, 2)

    @patch.object(release, 'gh')
    def test_creation_failure_is_not_silently_ignored(self, gh):
        gh.side_effect = [None, RuntimeError('tag creation rejected')]
        with self.assertRaisesRegex(RuntimeError, 'tag creation rejected'):
            release.ensure_tag('v0.0.2', 'validated-source')


if __name__ == '__main__':
    unittest.main()
