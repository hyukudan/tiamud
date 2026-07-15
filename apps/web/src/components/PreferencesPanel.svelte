<script lang="ts">
  import type { ClientPreferences } from '../lib/preferences';

  let {
    value,
    onchange,
  }: {
    value: ClientPreferences;
    onchange: (value: ClientPreferences) => void;
  } = $props();

  const update = <K extends keyof ClientPreferences>(key: K, next: ClientPreferences[K]) =>
    onchange({ ...value, [key]: next });
</script>

<details class="preferences-panel">
  <summary>Preferences</summary>
  <div>
    <label><input type="checkbox" checked={value.images} onchange={(event) => update('images', event.currentTarget.checked)} /> Images</label>
    <label><input type="checkbox" checked={value.audio} onchange={(event) => update('audio', event.currentTarget.checked)} /> Audio (off by default)</label>
    <label><input type="checkbox" checked={value.reduceMotion} onchange={(event) => update('reduceMotion', event.currentTarget.checked)} /> Reduce motion</label>
    <label><input type="checkbox" checked={value.saveData} onchange={(event) => update('saveData', event.currentTarget.checked)} /> Save data</label>
    <label><input type="checkbox" checked={value.highContrast} onchange={(event) => update('highContrast', event.currentTarget.checked)} /> High contrast</label>
    <label><input type="checkbox" checked={value.screenReader} onchange={(event) => update('screenReader', event.currentTarget.checked)} /> Screen-reader mode</label>

    <label>
      Text size
      <select value={value.fontScale} onchange={(event) => update('fontScale', Number(event.currentTarget.value) as ClientPreferences['fontScale'])}>
        <option value={100}>100%</option>
        <option value={112}>112%</option>
        <option value={125}>125%</option>
      </select>
    </label>

    {#if value.audio}
      <fieldset>
        <legend>Audio volumes</legend>
        <label>Music <input type="range" min="0" max="100" value={value.musicVolume} oninput={(event) => update('musicVolume', event.currentTarget.valueAsNumber)} /></label>
        <label>Ambience <input type="range" min="0" max="100" value={value.ambienceVolume} oninput={(event) => update('ambienceVolume', event.currentTarget.valueAsNumber)} /></label>
        <label>Effects <input type="range" min="0" max="100" value={value.effectsVolume} oninput={(event) => update('effectsVolume', event.currentTarget.valueAsNumber)} /></label>
      </fieldset>
    {/if}
    <p>Save data and screen-reader mode prevent scene artwork from being requested.</p>
  </div>
</details>
