;;; jaunder-warn.el --- Jaunder publish-time warning primitive -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;; This program is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.
;;
;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;;
;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:
;; The publish-time warning primitive.  `jaunder--warn' is the one place the
;; warning type, level, and "jaunder: " message prefix live, so every warning
;; reads the same in the *Warnings* buffer.  Warnings are soft and never block
;; the publish; the per-warning `jaunder-warn-*' toggles live in `jaunder-config'
;; and the domain-specific deciders live with their features (`jaunder-datetime',
;; `jaunder-media', `jaunder-service').

;;; Code:

(defun jaunder--warn (format-string &rest args)
  "Emit a soft jaunder publish warning; never blocks the publish.
FORMAT-STRING and ARGS are passed to `format'; the message is prefixed
\"jaunder: \" and reported via `display-warning' with type `jaunder' at
level `:warning'."
  (display-warning 'jaunder
                   (apply #'format (concat "jaunder: " format-string) args)
                   :warning))

(provide 'jaunder-warn)
;;; jaunder-warn.el ends here
