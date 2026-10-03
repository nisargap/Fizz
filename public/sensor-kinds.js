(() => {
  const SVG_NS = 'http://www.w3.org/2000/svg';
  // Icons are layered paths on a 16×16 pixel grid. A leading "-" marks a cutout painted in the badge background.
  const kinds = {
    water: { label: 'Water', detail: 'Leaks & flow', metric: 'flow_l_min', unit: 'L/min', color: '#5cc8ff', example: 'Basement leak', icon: ['M7 1h2v2h2v3h2v2h1v4h-1v2h-2v1H5v-1H3v-2H2V8h1V6h2V3h2z', '-M5 9h2v4H5z'] },
    gas: { label: 'Gas', detail: 'Air safety', metric: 'concentration_ppm', unit: 'ppm', color: '#9be07a', example: 'Garage air', icon: ['M5 4h6v2h2v2h2v4h-2v2H3v-2H1V8h2V6h2z', '-M6 7h2v2H6zm3 3h2v2H9z'] },
    radio: { label: 'Radio', detail: 'Signals & waves', metric: 'rssi_dbm', unit: 'dBm', color: '#ffe066', example: 'Rooftop antenna', icon: ['M7 7h2v7H7zM5 14h6v1H5zM3 5h2v2H3zm8 0h2v2h-2zM1 2h2v3H1zm12 0h2v3h-2zM5 3h2v2H5zm4 0h2v2H9z'] },
    temperature: { label: 'Temperature', detail: 'Hot & cold', metric: 'temperature_c', unit: '°C', color: '#ff7b6b', example: 'Kitchen temperature', icon: ['M6 1h4v9h2v4h-2v1H6v-1H4v-4h2z', '-M7 3h2v7H7z'] },
    pressure: { label: 'Pressure', detail: 'Force & level', metric: 'pressure_kpa', unit: 'kPa', color: '#c79bff', example: 'Boiler pressure', icon: ['M4 2h8v2h2v8h-2v2H4v-2H2V4h2z', '-M5 5h6v6H5z', 'M7 7h2v5H7zM9 6h2v2H9z'] },
    humidity: { label: 'Humidity', detail: 'Moisture', metric: 'humidity_pct', unit: '%', color: '#5ee6c4', example: 'Greenhouse humidity', icon: ['M4 1h2v2h2v3h1v3H8v2H6v-2H4V6H3V3h1zm7 5h2v2h1v3h-1v2h-2v-2h-1V8h1zM2 13h2v2H2zm4 0h2v2H6z'] },
    sound: { label: 'Sound', detail: 'Audio events', metric: 'sound_db', unit: 'dB', color: '#ffa94d', example: 'Nursery sound', icon: ['M2 6h3V4h2V2h2v12H7v-2H5v-2H2zM10 6h2v4h-2zm3-3h2v10h-2z'] },
    phone: { label: 'Phone', detail: 'Motion, voice & location', metric: 'acceleration_ms2', unit: 'm/s²', color: '#8c9dff', example: 'My phone', icon: ['M4 1h8v14H4z', '-M5 3h6v9H5zm2 10h2v1H7z'] },
    custom: { label: 'Custom', detail: 'Your own metric', metric: 'value', unit: '', color: '#ff9ed2', example: 'Bike rack counter', icon: ['M7 2h2v5h5v2H9v5H7V9H2V7h5z', 'M2 2h2v2H2zm10 0h2v2h-2zM2 12h2v2H2zm10 0h2v2h-2z'] },
  };
  const order = ['water', 'gas', 'radio', 'temperature', 'pressure', 'humidity', 'sound', 'phone', 'custom'];
  const fallback = { label: 'Sensor', detail: '', metric: 'value', unit: '', color: '#a9eaf7', example: 'My sensor', icon: kinds.custom.icon };

  const metricNames = {
    temperature_c: 'Temperature', flow_l_min: 'Flow rate', concentration_ppm: 'Gas concentration',
    rssi_dbm: 'Signal strength', pressure_kpa: 'Pressure', humidity_pct: 'Humidity', sound_db: 'Sound level',
    acceleration_ms2: 'Acceleration', acceleration_x: 'Acceleration X', acceleration_y: 'Acceleration Y', acceleration_z: 'Acceleration Z',
    rotation_alpha: 'Compass heading', rotation_beta: 'Front/back tilt', rotation_gamma: 'Left/right tilt', orientation_deg: 'Orientation',
    latitude: 'Latitude', longitude: 'Longitude', location_accuracy: 'Location accuracy',
    voice_clip: 'Voice clip', voice_level: 'Voice level', voice_transcript: 'Transcript', value: 'Value',
  };
  const numbers = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
  const relative = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });

  function get(kind) { return kinds[kind] || fallback; }

  function icon(kind) {
    const info = get(kind);
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('viewBox', '0 0 16 16');
    svg.setAttribute('shape-rendering', 'crispEdges');
    svg.setAttribute('aria-hidden', 'true');
    for (const layer of info.icon) {
      const path = document.createElementNS(SVG_NS, 'path');
      if (layer.startsWith('-')) path.setAttribute('class', 'icon-cutout');
      path.setAttribute('d', layer.replace(/^-/, ''));
      svg.append(path);
    }
    return svg;
  }

  // A square pixel badge coloured for the sensor kind.
  function badge(kind, className = 'kind-badge') {
    const element = document.createElement('span');
    element.className = className;
    element.style.setProperty('--sensor-color', get(kind).color);
    element.append(icon(kind));
    return element;
  }

  function metricLabel(metric) {
    if (!metric) return '';
    return metricNames[metric] || metric.replaceAll('_', ' ').replace(/^./, (letter) => letter.toUpperCase());
  }

  function formatValue(value, unit) {
    const text = typeof value === 'number' ? numbers.format(value) : String(value);
    if (!unit) return text;
    return unit === '%' ? `${text}%` : `${text} ${unit}`;
  }

  function formatReading(reading) {
    if (!reading) return 'Waiting for data';
    if (reading.metric === 'voice_clip') return 'Clip received';
    return formatValue(reading.numeric_value ?? reading.boolean_value ?? reading.text_value, reading.unit);
  }

  function timeAgo(value) {
    const seconds = Math.round((new Date(value).getTime() - Date.now()) / 1000);
    if (!Number.isFinite(seconds)) return '';
    if (Math.abs(seconds) < 45) return 'just now';
    if (Math.abs(seconds) < 3600) return relative.format(Math.round(seconds / 60), 'minute');
    if (Math.abs(seconds) < 86400) return relative.format(Math.round(seconds / 3600), 'hour');
    return relative.format(Math.round(seconds / 86400), 'day');
  }

  window.fizzKinds = { order, get, icon, badge, metricLabel, formatValue, formatReading, timeAgo };
})();
