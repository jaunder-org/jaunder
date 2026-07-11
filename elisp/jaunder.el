;;; jaunder.el --- Jaunder blogging client (AtomPub) -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;; Author: Jaunder contributors
;; Version: 0.1.0
;; Package-Requires: ((emacs "29.1"))
;; Keywords: hypermedia, comm, outlines
;; URL: https://jaunder.org

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
;; Publish and reconcile Org-mode blog posts against a Jaunder server over
;; AtomPub.  See `jaunder-blogs' to configure one or more blogs.

;;; Code:

(require 'jaunder-entry)
(require 'jaunder-config)
(require 'jaunder-warn)
(require 'jaunder-datetime)
(require 'jaunder-atom)
(require 'jaunder-org)
(require 'jaunder-transport)
(require 'jaunder-media)
(require 'jaunder-publish)

(provide 'jaunder)
;;; jaunder.el ends here
