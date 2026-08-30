;;; jaunder.el --- Jaunder blogging client (AtomPub) -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;; Author: Jaunder contributors
;; Version: 0.1.0
;; Package-Requires: ((emacs "29.1") (cmark "0.29.3") (plz "0.9.1"))
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

(require 'jaunder-entry) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-config) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-warn) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-datetime) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-atom) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-org) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-transport) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-service) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-media) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-publish) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-reconcile) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-delete) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-pull) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop
(require 'jaunder-pull-media) ;; cov:ignore: load-time dependency declaration has no Edebug execution stop

(provide 'jaunder) ;; cov:ignore: feature declaration has no Edebug execution stop
;;; jaunder.el ends here
