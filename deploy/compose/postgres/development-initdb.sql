-- FND-2 / CPR-45: extension prerequisites for the retained contributor-only
-- database. The `development` image target is its sole caller; reference and
-- release deployments install and prove these through database convergence.
create extension if not exists vector;
create extension if not exists btree_gin;
