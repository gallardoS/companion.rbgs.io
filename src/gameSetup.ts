import type { useSavedVariables } from './SavedVariables';

export type SettingsTarget = 'account' | 'uploads' | 'folder' | 'wow-account' | 'addon';

export function gameSetup(source: ReturnType<typeof useSavedVariables>) {
  const { view, error, watchError } = source;
  const game = view?.game;
  const folderReady = !!view?.installation && !['NOT_CONFIGURED', 'WRONG_CLIENT'].includes(game?.status ?? 'NOT_CONFIGURED');
  const selectedAccount = view?.sources.find(account => account.id === view.selectedSource);
  const readerFailure = error !== null || watchError !== null;
  const needsSourceSelection = !selectedAccount || ['SELECT_SOURCE', 'SOURCE_UNAVAILABLE'].includes(game?.status ?? '');
  const ready = folderReady && !needsSourceSelection && game?.status === 'READY' && !readerFailure;
  let title = 'Addon ready';
  let description = '';
  let target: SettingsTarget = 'addon';

  if (readerFailure) {
    title = 'Cannot read saved game data';
    description = 'Check your WoW folder and account, then try reading the addon data again.';
  } else if (!view) {
    title = 'Looking for WoW';
    description = 'Checking your folder and saved addon data.';
    target = 'folder';
  } else if (!folderReady) {
    title = 'Choose your WoW folder';
    description = game?.status === 'WRONG_CLIENT' ? 'Select the _classic_beta_ folder from your Forever beta installation.' : 'Select your Forever beta installation to get started.';
    target = 'folder';
  } else if (needsSourceSelection) {
    if (!view.sources.length) {
      title = 'Save your WoW account data';
      description = 'No WoW accounts found in this installation. Log into a character, use /reload or log out, then check again.';
      target = 'folder';
    } else {
      title = 'Choose your WoW account';
      description = game?.status === 'SOURCE_UNAVAILABLE' ? 'The selected account data could not be read. Check the folder and account, then try again.' : 'Select the WoW account you use to play.';
      target = 'wow-account';
    }
  } else if (!ready) {
    const copy: Record<string, [string, string]> = {
      MISSING_ADDON: ['Install the rbgs.io addon', 'Install and enable rbgs.io in WoW, then log into your character and use /reload.'],
      UNSUPPORTED_ADDON: ['Update the rbgs.io addon', 'Install the latest rbgs.io addon, then use /reload in WoW.'],
      NO_SAVED_DATA: ['Save your addon data', 'Log into your character, then use /reload or log out to save addon data.'],
      UNSTABLE_FILE: ['Waiting for WoW to finish saving', 'The addon file is changing. The companion will read it when saving finishes.'],
    };
    [title, description] = copy[game?.status ?? ''] ?? ['Check your addon data', game?.message ?? 'Check the selected folder and account, then try reading addon data again.'];
  }
  return { folderReady, selectedAccount, readerFailure, needsSourceSelection, ready, title, description, target };
}
