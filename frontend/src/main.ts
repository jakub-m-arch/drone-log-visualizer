import { mount } from 'svelte'
import 'maplibre-gl/dist/maplibre-gl.css'
import 'uplot/dist/uPlot.min.css'
import './app.css'
import App from './App.svelte'

const app = mount(App, { target: document.getElementById('app')! })

export default app
