-- SPDX-License-Identifier: GPL-3.0-or-later
function love.load()
  love.window.setTitle("minde SDL2 compatibility")
  love.window.setMode(480, 280)
end

function love.draw()
  love.graphics.print("SDL2 / Wayland", 40, 40)
end
